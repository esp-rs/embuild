use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Error, Result};
use serde::Deserialize;

use super::index::{self, ObjKind};
use super::Version;

#[derive(Debug, Deserialize, Clone)]
pub struct Codemodel {
    #[serde(skip)]
    codemodel_dir: Arc<PathBuf>,
    pub version: Version,
    pub paths: Paths,
    pub configurations: Vec<Configuration>,
}

impl TryFrom<&index::Reply> for Codemodel {
    type Error = Error;
    fn try_from(value: &index::Reply) -> Result<Self, Self::Error> {
        assert!(value.kind == ObjKind::Codemodel);
        ObjKind::Codemodel
            .check_version_supported(value.version.major)
            .unwrap();

        let mut codemodel: Codemodel = serde_json::from_reader(&fs::File::open(&value.json_file)?)
            .with_context(|| {
                anyhow!(
                    "Parsing cmake-file-api codemodel object file '{}' failed",
                    value.json_file.display()
                )
            })?;

        codemodel.codemodel_dir = Arc::new(value.json_file.parent().unwrap().to_owned());
        for conf in codemodel.configurations.iter_mut() {
            conf.codemodel_dir = codemodel.codemodel_dir.clone();
        }

        Ok(codemodel)
    }
}

impl Codemodel {
    pub fn into_conf(self) -> Vec<Configuration> {
        self.configurations
    }

    pub fn into_first_conf(self) -> Configuration {
        self.configurations
            .into_iter()
            .next()
            .expect("no configurations")
    }

    /// The path to the directory containing the file represented by this
    /// [`Codemodel`] instance.
    pub fn dir_path(&self) -> &PathBuf {
        &self.codemodel_dir
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Paths {
    /// The absolute path to the top-level source directory.
    pub source: PathBuf,
    /// The absolute path to the top-level build directory.
    pub build: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Configuration {
    #[serde(skip)]
    codemodel_dir: Arc<PathBuf>,
    pub name: String,
    #[serde(rename = "targets")]
    pub target_refs: Vec<TargetRef>,
}

impl Configuration {
    pub fn get_target(&self, name: impl AsRef<str>) -> Option<Result<target::Target>> {
        self.target_refs
            .iter()
            .find(|t| t.name == name.as_ref())
            .map(|t| t.deref(self))
    }

    pub fn targets(&self) -> impl Iterator<Item = Result<target::Target>> + '_ {
        self.target_refs.iter().map(move |t| t.deref(self))
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetRef {
    pub name: String,
    pub directory_index: usize,
    pub project_index: usize,
    pub json_file: String,
}

impl TargetRef {
    pub fn deref(&self, cfg: &Configuration) -> Result<target::Target> {
        target::Target::from_file(cfg.codemodel_dir.join(&self.json_file))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Language {
    C,
    #[serde(rename = "CXX")]
    Cpp,
    #[serde(rename = "CUDA")]
    Cuda,
    #[serde(rename = "OBJCXX")]
    ObjectiveCpp,
    #[serde(rename = "HIP")]
    Hip,
    #[serde(rename = "ISPC")]
    Ispc,
    #[serde(rename = "ASM")]
    Assembly,
}

pub use target::Target;

pub mod target {
    use std::path::{Path, PathBuf};

    use anyhow::*;
    use serde::Deserialize;

    use super::Language;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Hash)]
    #[serde(rename_all = "SCREAMING_SNAKE_CASE")]
    pub enum Type {
        Executable,
        StaticLibrary,
        SharedLibrary,
        ModuleLibrary,
        ObjectLibrary,
        InterfaceLibrary,
        Utility,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Target {
        pub name: String,
        pub link: Link,
        pub compile_groups: Vec<CompileGroup>,
        #[serde(rename = "type")]
        pub target_type: Type,
    }

    impl Target {
        pub fn from_file(file_path: impl AsRef<Path>) -> Result<Target> {
            let file = std::fs::File::open(&file_path)?;
            let value: Target = serde_json::from_reader(file).with_context(|| {
                anyhow!(
                    "Failed to parse the cmake-file-api target file '{}'",
                    file_path.as_ref().display()
                )
            })?;

            Ok(value)
        }
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct CompileGroup {
        pub language: Language,
        #[serde(default)]
        pub compile_command_fragments: Vec<Fragment>,
        #[serde(default)]
        pub includes: Vec<Include>,
        #[serde(default)]
        pub defines: Vec<Define>,
        pub sysroot: Option<Sysroot>,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct Fragment {
        pub fragment: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct Define {
        pub define: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Include {
        pub path: String,
        #[serde(default)]
        pub is_system: bool,
    }

    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Link {
        pub language: String,
        pub command_fragments: Vec<CommandFragment>,
        #[serde(default)]
        pub lto: bool,
        pub sysroot: Option<Sysroot>,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct CommandFragment {
        pub fragment: String,
        pub role: Role,
    }

    #[derive(Debug, PartialEq, Eq, Deserialize, Clone, Copy)]
    #[serde(rename_all = "camelCase")]
    pub enum Role {
        /// Link flags
        Flags,
        /// Link library file paths or flags
        Libraries,
        /// Library search path flags
        LibraryPath,
        /// MacOS framework search path flags
        FrameworkPath,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct Sysroot {
        pub path: PathBuf,
    }
}
