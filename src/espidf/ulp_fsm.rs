use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::{env, iter};

use crate::build::CInclArgs;
use crate::utils::OsStrExt;
use crate::{symgen, *};

#[derive(Clone, Debug)]
pub enum SystemIncludes {
    CInclArgs(CInclArgs),
    MCU(String),
}

#[derive(Clone, Debug)]
pub struct BuildResult {
    pub bin_file: PathBuf,
    pub elf_file: PathBuf,
    pub sym_rs_file: PathBuf,
}

pub struct Builder {
    esp_idf: PathBuf,
    sys_includes: SystemIncludes,
    add_includes: Vec<String>,
    gcc: Option<String>,
    env_path: Option<OsString>,
}

impl Builder {
    pub fn try_from_embuild_env(
        library: impl AsRef<str>,
        add_includes: impl Into<Vec<String>>,
    ) -> anyhow::Result<Self> {
        let library = library.as_ref();

        Ok(Self {
            esp_idf: PathBuf::from(env::var(format!("DEP_{library}_EMBUILD_ESP_IDF_PATH"))?),
            sys_includes: SystemIncludes::CInclArgs(build::CInclArgs::try_from_env(library)?),
            add_includes: add_includes.into(),
            gcc: None,
            env_path: env::var_os("DEP_ESP_IDF_EMBUILD_ENV_PATH"),
        })
    }

    pub fn new(
        esp_idf: impl Into<PathBuf>,
        sys_includes: SystemIncludes,
        add_includes: impl Into<Vec<String>>,
        gcc: Option<String>,
        env_path: Option<OsString>,
    ) -> Self {
        Self {
            esp_idf: esp_idf.into(),
            sys_includes,
            add_includes: add_includes.into(),
            gcc,
            env_path,
        }
    }

    pub fn build<'a, I>(
        &self,
        ulp_sources: I,
        out_dir: impl AsRef<Path>,
    ) -> anyhow::Result<BuildResult>
    where
        I: IntoIterator<Item = &'a Path>,
    {
        let out_dir = out_dir.as_ref();

        let include_args = self.include_args();

        let ulp_obj_out_dir = path_buf![&out_dir, "obj"];

        self.compile(ulp_sources, &include_args, &ulp_obj_out_dir)?;

        let ulp_ld_script = ["ulp_fsm.ld", "esp32.ulp.ld"]
            .into_iter()
            .map(|ulp_file_name| path_buf![&self.esp_idf, "components", "ulp", "ld", ulp_file_name])
            .find(|ulp_path| ulp_path.exists())
            .ok_or_else(|| anyhow::anyhow!("Cannot find the ULP FSM LD script in ESP-IDF"))?;

        let ulp_ld_out_script = path_buf![&out_dir, "ulp.ld"];

        self.preprocess_one(&ulp_ld_script, &include_args, &ulp_ld_out_script)?;

        let ulp_elf = path_buf![&out_dir, "ulp"];

        self.link(&ulp_obj_out_dir, &ulp_ld_out_script, &ulp_elf)?;

        let ulp_bin = path_buf![&out_dir, "ulp.bin"];

        self.bin(&ulp_elf, &ulp_bin)?;

        let ulp_sym_rs = path_buf![&out_dir, "ulp.rs"];

        self.symbolize(&ulp_elf, &ulp_sym_rs)?;

        Ok(BuildResult {
            bin_file: ulp_bin,
            elf_file: ulp_elf,
            sym_rs_file: ulp_sym_rs,
        })
    }

    fn compile<'a, I>(
        &self,
        ulp_sources: I,
        include_args: &[impl AsRef<OsStr>],
        out_dir: &Path,
    ) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = &'a Path>,
    {
        for ulp_source in ulp_sources {
            std::fs::create_dir_all(out_dir)?;

            let ulp_preprocessed_source = Self::resuffix(ulp_source, out_dir, "ulp.S")?;

            self.preprocess_one(ulp_source, include_args, &ulp_preprocessed_source)?;

            let ulp_object = Self::resuffix(ulp_source, out_dir, "o")?;

            self.compile_one(&ulp_preprocessed_source, &ulp_object)?;
        }

        Ok(())
    }

    fn compile_one(&self, ulp_source: &Path, out_file: &Path) -> anyhow::Result<()> {
        cmd![self.tool("esp32ulp-elf-as")?, "-o", out_file, ulp_source].run()?;

        Ok(())
    }

    fn preprocess_one(
        &self,
        source: &Path,
        include_args: &[impl AsRef<OsStr>],
        out_file: &Path,
    ) -> anyhow::Result<()> {
        cmd![
            self.tool(self.gcc.as_deref().unwrap_or("xtensa-esp32-elf-gcc"))?,
            "-E",
            "-P",
            "-xc",
            "-D__ASSEMBLER__",
            @include_args,
            "-o",
            out_file,
            source
        ]
        .run()?;

        Ok(())
    }

    fn link(
        &self,
        object_files_dir: &Path,
        linker_script: &Path,
        out_file: &Path,
    ) -> anyhow::Result<()> {
        let object_files = std::fs::read_dir(object_files_dir)?
            .filter_map(|file| {
                file.ok()
                    .filter(|file| file.path().extension().map(|e| e == "o").unwrap_or(false))
            })
            .map(|de| de.path().as_os_str().to_owned())
            .collect::<Vec<_>>();

        cmd![
            self.tool("esp32ulp-elf-ld")?,
            "-T",
            linker_script,
            @object_files,
            "-o",
            out_file
        ]
        .run()?;

        Ok(())
    }

    fn bin(&self, ulp_elf: &Path, out_file: &Path) -> anyhow::Result<()> {
        // TODO: Switch to our own bingen in embuild
        cmd![
            self.tool("esp32ulp-elf-objcopy")?,
            ulp_elf,
            "-O",
            "binary",
            out_file
        ]
        .run()?;

        Ok(())
    }

    fn symbolize(&self, ulp_elf: &Path, out_file: &Path) -> anyhow::Result<()> {
        symgen::Symgen::new_with_pointer_gen(ulp_elf, 0x5000_0000_u64, |symbol| {
            symbol
                .sections(&[
                    symgen::Section::code(".text"),
                    symgen::Section::data(".bss"),
                    symgen::Section::data(".data"),
                ])
                .map(|mut pointer| {
                    pointer.r#type = Some("u32".to_owned());
                    pointer
                })
        })
        .run_for_file(out_file)
    }

    fn include_args(&self) -> Vec<String> {
        match self.sys_includes {
            SystemIncludes::CInclArgs(ref include_args) => self
                .add_includes
                .iter()
                .cloned()
                .chain(
                    include_args
                        .args
                        .split_ascii_whitespace()
                        .filter_map(Self::unescape),
                )
                .collect::<Vec<_>>(),
            SystemIncludes::MCU(ref mcu) => self
                .add_includes
                .iter()
                .cloned()
                .chain(iter::once(format!(
                    "{}",
                    path_buf![&self.esp_idf, "components", "soc", mcu].display()
                )))
                .flat_map(|s| iter::once("-I".to_owned()).chain(iter::once(s)))
                .collect::<Vec<_>>(),
        }
    }

    fn unescape(arg: &str) -> Option<String> {
        let unescaped = if arg.starts_with('\"') && arg.ends_with('\"') {
            arg[1..arg.len() - 1].replace("\\\"", "\"")
        } else {
            arg.to_owned()
        };

        if unescaped.starts_with("-isystem") || unescaped.starts_with("-I") {
            Some(unescaped)
        } else {
            None
        }
    }

    fn resuffix(path: &Path, out_dir: &Path, suffix: &str) -> anyhow::Result<PathBuf> {
        let resuffixed = path_buf![
            &out_dir,
            format!(
                "{}.{}",
                path.file_stem()
                    .ok_or_else(|| anyhow::anyhow!("Wrong file name {}", path.display()))?
                    .try_to_str()?,
                suffix
            )
        ];

        Ok(resuffixed)
    }

    fn tool(&self, tool: &str) -> anyhow::Result<PathBuf> {
        let path = which::which_in(tool, self.env_path.clone(), env::current_dir()?)?;

        Ok(path)
    }
}
