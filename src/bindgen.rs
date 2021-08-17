use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use anyhow::*;

use crate::cargo;
use crate::pio::project::SconsVariables;
use crate::utils::OsStrExt;

pub const VAR_BINDINGS_FILE: &'static str = "CARGO_PIO_BINDGEN_RUNNER_BINDINGS_FILE";

#[cfg(windows)]
const EXE_SUFFIX: &'static str = ".exe";

#[cfg(not(windows))]
const EXE_SUFFIX: &'static str = "";

#[cfg(windows)]
const FS_CASE_INSENSITIVE: bool = true;

#[cfg(not(windows))]
const FS_CASE_INSENSITIVE: bool = false;

#[derive(Clone, Default, Debug)]
pub struct Factory {
    pub clang_args: Vec<String>,
    pub linker: Option<PathBuf>,
    pub mcu: Option<String>,
}

impl Factory {
    pub fn from_scons_vars(scons_vars: &SconsVariables) -> Result<Self> {
        Ok(Self {
            clang_args: Self::get_pio_clang_args(
                &scons_vars.incflags,
                scons_vars.clangargs.clone(),
            ),
            linker: Some(scons_vars.full_path(scons_vars.link.clone())?),
            mcu: Some(scons_vars.mcu.clone()),
        })
    }

    pub fn builder(&self) -> Result<bindgen::Builder> {
        self.create_builder(false)
    }

    pub fn cpp_builder(&self) -> Result<bindgen::Builder> {
        self.create_builder(true)
    }

    fn create_builder(&self, cpp: bool) -> Result<bindgen::Builder> {
        let sysroot = self.get_sysroot()?;

        let builder = bindgen::Builder::default()
            .use_core()
            .layout_tests(false)
            .rustfmt_bindings(false)
            .derive_default(true)
            //.ctypes_prefix(c_types)
            .clang_arg("-D__bindgen")
            .clang_arg(format!("--sysroot={}", sysroot.display()))
            .clang_arg(format!("-I{}", sysroot.join("include").try_to_str()?))
            .clang_args(&["-x", if cpp { "c++" } else { "c" }])
            .clang_args(if cpp {
                Self::get_cpp_includes(sysroot)?
            } else {
                Vec::new()
            })
            .clang_args(&self.clang_args);

        eprintln!(
            "Bindgen builder factory flags: {:?}",
            builder.command_line_flags()
        );

        Ok(builder)
    }

    fn get_sysroot(&self) -> Result<PathBuf> {
        let linker = if let Some(linker) = self.linker.as_ref() {
            linker
                .clone()
                .into_os_string()
                .into_string()
                .map_err(|_| anyhow!("Cannot convert the linker variable to String"))?
        } else if let Ok(linker) = env::var("RUSTC_LINKER") {
            linker
        } else {
            bail!("No explicit linker, and env var RUSTC_LINKER not defined either");
        };

        let gcc = format!("gcc{}", EXE_SUFFIX);
        let gcc_suffix = format!("-{}", gcc);

        let linker_canonicalized = if FS_CASE_INSENSITIVE {
            linker.to_lowercase()
        } else {
            linker.clone()
        };

        let linker = if linker_canonicalized == gcc || linker_canonicalized.ends_with(&gcc_suffix) {
            // For whatever reason, --print-sysroot does not work with GCC
            // Change it to LD
            format!("{}ld{}", &linker[0..linker.len() - gcc.len()], EXE_SUFFIX)
        } else {
            linker
        };

        let output = Command::new(linker).arg("--print-sysroot").output()?;

        let path_str = String::from_utf8(output.stdout)?;

        Ok(PathBuf::from(path_str.trim()))
    }

    fn get_cpp_includes(sysroot: impl AsRef<Path>) -> Result<Vec<String>> {
        let sysroot = sysroot.as_ref();
        let cpp_includes_root = sysroot.join("include").join("c++");

        let cpp_version = fs::read_dir(&cpp_includes_root)?
            .map(|dir_entry_r| dir_entry_r.map(|dir_entry| dir_entry.path()))
            .fold(None, |ao: Option<PathBuf>, sr: Result<PathBuf, _>| {
                if let Some(a) = ao.as_ref() {
                    sr.ok()
                        .map_or(ao.clone(), |s| if a >= &s { ao.clone() } else { Some(s) })
                } else {
                    sr.ok()
                }
            });

        if let Some(cpp_version) = cpp_version {
            let mut cpp_include_paths = vec![
                format!("-I{}", cpp_version.try_to_str()?),
                format!("-I{}", cpp_version.join("backward").try_to_str()?),
            ];

            if let Some(sysroot_last_segment) = fs::canonicalize(sysroot)?.file_name() {
                cpp_include_paths.push(format!(
                    "-I{}",
                    cpp_version.join(sysroot_last_segment).try_to_str()?
                ));
            }

            Ok(cpp_include_paths)
        } else {
            Ok(Vec::new())
        }
    }

    fn get_pio_clang_args(
        incflags: impl AsRef<str>,
        extra_args: Option<impl AsRef<str>>,
    ) -> Vec<String> {
        let mut result = incflags
            .as_ref()
            .split(' ')
            .map(str::to_string)
            .collect::<Vec<_>>();

        if let Some(extra_args) = extra_args {
            result.append(
                &mut extra_args
                    .as_ref()
                    .split(' ')
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
            );
        }

        result
    }
}

pub fn run(builder: bindgen::Builder) -> Result<()> {
    let output_file = PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs");

    run_for_file(builder, &output_file)?;

    println!(
        "cargo:rustc-env={}={}",
        VAR_BINDINGS_FILE,
        output_file.display()
    );

    Ok(())
}

pub fn run_for_file(builder: bindgen::Builder, output_file: impl AsRef<Path>) -> Result<()> {
    let output_file = output_file.as_ref();

    eprintln!("Output: {:?}", output_file);
    eprintln!("Bindgen builder flags: {:?}", builder.command_line_flags());

    let bindings = builder
        .generate()
        .map_err(|_| Error::msg("Failed to generate bindings"))?;

    bindings.write_to_file(output_file)?;

    // Run rustfmt on the generated bindings separately, because custom toolchains often do not have rustfmt
    // Hence why we need to use the rustfmt from the stable buildchain (where the assumption is, it is already installed)
    Command::new("rustup")
        .arg("run")
        .arg("stable")
        .arg("rustfmt")
        .arg(output_file)
        .status()?;

    Ok(())
}
