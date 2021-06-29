use std::{env, fs, path::{Path, PathBuf}, process::Command};

use anyhow::*;

use crate::SconsVariables;

pub const VAR_BINDINGS_FILE: &'static str = "CARGO_PIO_BINDGEN_RUNNER_BINDINGS_FILE";

#[cfg(windows)]
const EXE_SUFFIX: &'static str = ".exe";

#[cfg(not(windows))]
const EXE_SUFFIX: &'static str = "";

#[cfg(windows)]
const FS_CASE_INSENSITIVE: bool = true;

#[cfg(not(windows))]
const FS_CASE_INSENSITIVE: bool = false;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Language {
    C,
    CPlusPlus
}

#[derive(Clone, Default, Debug)]
pub struct Runner {
    pub should_generate: bool,
    pub clang_args: Vec<String>,
    pub linker: Option<PathBuf>,
    pub mcu: Option<String>,
}

impl Runner {
    pub fn from_scons_vars(scons_vars: &SconsVariables) -> Result<Self> {
        Ok(Self {
            should_generate: true,
            clang_args: Self::get_pio_clang_args(&scons_vars.incflags, scons_vars.clangargs.clone()),
            linker: Some(scons_vars.full_path(scons_vars.link.clone())?),
            mcu: Some(scons_vars.mcu.clone()),
        })
    }

    pub fn run(&self, bindings_headers: &[impl AsRef<str>], language: Language) -> Result<()> {
        self.run_with_builder_options(bindings_headers, language, |_, builder| builder)
    }

    pub fn run_with_builder_options(&self,
            bindings_headers: &[impl AsRef<str>],
            language: Language,
            builder_options_factory: impl FnOnce(&Path, bindgen::Builder) -> bindgen::Builder) -> Result<()> {
        if self.should_generate {
            let sysroot = self.get_sysroot()?;

            let builder = self.create_builder(&sysroot, bindings_headers, language)?;

            let builder = builder_options_factory(&sysroot, builder);

            let bindings = builder
                .generate()
                .map_err(|_| Error::msg("Failed to generate bindings"))?;

            let bindings_file = Self::write_bindings(bindings)?;

            self.output_cargo_instructions(bindings_headers, bindings_file);
        } else {
            self.output_cargo_instructions_for_pregenerated();
        }

        Ok(())
    }

    fn create_builder(
            &self,
            sysroot: impl AsRef<Path>,
            bindings_headers: &[impl AsRef<str>],
            language: Language) -> Result<bindgen::Builder> {
        let sysroot = sysroot.as_ref();

        let mut builder = bindgen::Builder::default()
            .use_core()
            .layout_tests(false)
            .rustfmt_bindings(false)
            .derive_default(true)
            .ctypes_prefix("c_types"/*"libc"*/)
            .clang_arg("-D__bindgen")
            .clang_arg(format!("--sysroot={}", sysroot.display()))
            .clang_args(&["-x", if language == Language::CPlusPlus {"c++"} else {"c"}])
            .clang_args(if language == Language::CPlusPlus {Self::get_cpp_includes(sysroot)?} else {Vec::new()})
            .clang_arg(format!("-I{}", Self::to_string(sysroot.join("include"))?))
            .clang_args(&self.clang_args);

        for header in bindings_headers {
            builder = builder.header(header.as_ref());
        }

        eprintln!("Bindgen flags: {:?}", builder.command_line_flags());

        Ok(builder)
    }

    fn get_sysroot(&self) -> Result<PathBuf> {
        let linker = if let Some(linker) = self.linker.as_ref() {
            linker.clone().into_os_string().into_string().map_err(|_| anyhow!("Cannot convert the linker variable to String"))?
        } else if let Ok(linker) = env::var("RUSTC_LINKER") {
            linker
        } else {
            bail!("No explicit linker, and env var RUSTC_LINKER not defined either");
        };

        let gcc = format!("gcc{}", EXE_SUFFIX);
        let gcc_suffix = format!("-{}", gcc);

        let linker_canonicalized = if FS_CASE_INSENSITIVE {linker.to_lowercase()} else {linker.clone()};

        let linker = if linker_canonicalized == gcc || linker_canonicalized.ends_with(&gcc_suffix) {
            // For whatever reason, --print-sysroot does not work with GCC
            // Change it to LD
            format!("{}ld{}", &linker[0..linker.len() - gcc.len()], EXE_SUFFIX)
        } else {
            linker
        };

        let output = Command::new(linker)
            .arg("--print-sysroot")
            .output()?;

        let path_str = String::from_utf8(output.stdout)?;

        Ok(fs::canonicalize(PathBuf::from(path_str.trim()).canonicalize()?)?)
    }

    fn get_cpp_includes(sysroot: impl AsRef<Path>) -> Result<Vec<String>> {
        let sysroot = sysroot.as_ref();
        let cpp_includes_root = sysroot.join("include").join("c++");

        let cpp_version = fs::read_dir(&cpp_includes_root)?
            .map(|dir_entry_r| dir_entry_r.map(|dir_entry| dir_entry.path()))
            .fold(None, |ao: Option<PathBuf>, sr: Result<PathBuf, _>| if let Some(a) = ao.as_ref() {
                sr.ok().map_or(
                    ao.clone(),
                    |s| if a >= &s {ao.clone()} else {Some(s)})
            } else {
                sr.ok()
            });

        if let Some(cpp_version) = cpp_version {
            let mut cpp_include_paths = vec![
                format!("-I{}", Self::to_string(&cpp_version)?),
                format!("-I{}", Self::to_string(cpp_version.join("backward"))?),
            ];

            if let Some(sysroot_last_segment) = fs::canonicalize(sysroot)?.file_name() {
                cpp_include_paths.push(format!("-I{}", Self::to_string(cpp_version.join(sysroot_last_segment))?));
            }

            Ok(cpp_include_paths)
        } else {
            Ok(Vec::new())
        }
    }

    fn write_bindings(bindings: bindgen::Bindings) -> Result<PathBuf> {
        let output_file = PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs");
        eprintln!("Output: {:?}", &output_file);

        bindings.write_to_file(&output_file)?;

        // Run rustfmt on the generated bindings separately, because custom toolchains often do not have rustfmt
        // Hence why we need to use the rustfmt from the stable buildchain (where the assumption is, it is already installed)
        Command::new("rustup")
            .arg("run")
            .arg("stable")
            .arg("rustfmt")
            .arg(&output_file)
            .status()?;

        Ok(output_file)
    }

    fn output_cargo_instructions(&self, bindings_headers: &[impl AsRef<str>], bindings_file: impl AsRef<Path>) {
        // TODO: println!("cargo:rerun-if-changed={}/sdkconfig.h", idf_bindings_header_dir);

        for header in bindings_headers {
            println!("cargo:rerun-if-changed={}", header.as_ref());
        }

        println!("cargo:rustc-env={}={}", VAR_BINDINGS_FILE, bindings_file.as_ref().display());
    }

    fn output_cargo_instructions_for_pregenerated(&self) {
        if let Some(mcu) = self.mcu.as_ref() {
            println!("cargo:warning=Using pre-generated bindings for MCU '{}'", mcu);
            println!("cargo:rustc-env={}=bindings_{}.rs", VAR_BINDINGS_FILE, mcu);
        } else {
            println!("cargo:warning=Using pre-generated bindings");
            println!("cargo:rustc-env={}=bindings.rs", VAR_BINDINGS_FILE);
        }
    }

    fn get_pio_clang_args(incflags: impl AsRef<str>, extra_args: Option<impl AsRef<str>>) -> Vec<String> {
        let mut result = incflags.as_ref()
            .split(' ')
            .map(str::to_string)
            .collect::<Vec<_>>();

        if let Some(extra_args) = extra_args {
            result.append(&mut extra_args.as_ref()
                .split(' ')
                .map(str::to_string)
                .collect::<Vec<_>>());
        }

        result
    }

    fn to_string(path: impl AsRef<Path>) -> Result<String> {
        path
            .as_ref()
            .to_str()
            .ok_or(Error::msg("Cannot convert to str"))
            .map(str::to_owned)
    }
}
