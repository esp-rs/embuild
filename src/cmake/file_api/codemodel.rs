//! Codemodel cmake file API object.
//!
//! The codemodel object kind describes the build system structure as modeled by CMake.

use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Error, Result};
use serde::Deserialize;

use super::index::{self, ObjKind};
use super::Version;

/// The description of the build system structure as modeled by CMake.
#[derive(Debug, Deserialize, Clone)]
pub struct Codemodel {
    #[serde(skip)]
    codemodel_dir: Arc<PathBuf>,
    /// The version of this object kind.
    pub version: Version,
    /// Some paths used by cmake.
    pub paths: Paths,
    /// All available build configurations.
    ///
    /// On single-configuration generators there is one entry for the value of the
    /// `CMAKE_BUILD_TYPE` variable. For multi-configuration generators there is an entry for
    /// each configuration listed in the `CMAKE_CONFIGURATION_TYPES` variable.
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
    /// Turn this into [`configurations`](Self::configurations).
    pub fn into_conf(self) -> Vec<Configuration> {
        self.configurations
    }

    /// Turn this into [`configurations[0]`](Self::configurations).
    ///
    /// This functions panics if there are no configurations.
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

/// Paths used by cmake.
#[derive(Debug, Deserialize, Clone)]
pub struct Paths {
    /// The absolute path to the top-level source directory.
    pub source: PathBuf,
    /// The absolute path to the top-level build directory.
    pub build: PathBuf,
}

/// A build configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Configuration {
    #[serde(skip)]
    codemodel_dir: Arc<PathBuf>,
    /// The name of the configuration (e.g. `Debug`)
    pub name: String,
    /// A build system target.
    ///
    /// Such targets are created by calls to `add_executable()`, `add_library()`, and
    /// `add_custom_target()`, excluding imported targets and interface libraries (which do
    /// not generate any build rules).
    #[serde(rename = "targets")]
    pub target_refs: Vec<TargetRef>,
}

impl Configuration {
    /// Load a codemodel target object by name.
    pub fn get_target(&self, name: impl AsRef<str>) -> Option<Result<target::Target>> {
        self.target_refs
            .iter()
            .find(|t| t.name == name.as_ref())
            .map(|t| t.load(self))
    }

    /// Load all codemodel target objects.
    pub fn targets(&self) -> impl Iterator<Item = Result<target::Target>> + '_ {
        self.target_refs.iter().map(move |t| t.load(self))
    }
}

/// A reference to a codemodel target object JSON file.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetRef {
    /// The target name.
    pub name: String,
    /// An unsigned integer 0-based index into the main directories array indicating the
    /// build system directory in which the target is defined.
    pub directory_index: usize,
    /// An unsigned integer 0-based index into the main projects array indicating the
    /// build system project in which the target is defined.
    pub project_index: usize,
    /// A path relative to the codemodel file to another JSON file containing a
    /// codemodel `target` object.
    pub json_file: String,
}

impl TargetRef {
    /// Load the target object from the [`json_file`](Self::json_file).
    pub fn load(&self, cfg: &Configuration) -> Result<target::Target> {
        target::Target::from_file(cfg.codemodel_dir.join(&self.json_file))
    }
}

/// A cmake supported programming language
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

/// Codemodel target cmake file API object.
pub mod target {
    use std::path::{Path, PathBuf};

    use anyhow::{anyhow, Context, Result};
    use serde::Deserialize;

    use super::Language;

    /// A type of cmake target.
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
        /// The logical name of the target.
        pub name: String,
        /// Link info of target.
        pub link: Option<Link>,
        /// Compile settings for source files.
        #[serde(default)]
        pub compile_groups: Vec<CompileGroup>,
        /// The type of the target.
        #[serde(rename = "type")]
        pub target_type: Type,
    }

    impl Target {
        /// Deserialize the codemodel target object JSON file from `file_path`.
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

    /// Compile settings for groups of sources using the same settings.
    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct CompileGroup {
        /// Language of the toolchain in use to compile the source file.
        pub language: Language,
        /// Fragments of the compiler command line invocation if available.
        #[serde(default)]
        pub compile_command_fragments: Vec<Fragment>,
        /// Include directories.
        #[serde(default)]
        pub includes: Vec<Include>,
        /// Prerocessor definitions.
        #[serde(default)]
        pub defines: Vec<Define>,
        /// Path to the sysroot.
        ///
        /// Present when the `CMAKE_SYSROOT_COMPILE` or `CMAKE_SYSROOT` variable is defined.
        pub sysroot: Option<Sysroot>,
    }

    /// A fragment of a compile command invocation.
    #[derive(Debug, Deserialize, Clone)]
    pub struct Fragment {
        /// A fragment of the compile command line invocation.
        ///
        /// The value is encoded in the build system's native shell format.
        pub fragment: String,
    }

    /// A preprocessor definition.
    #[derive(Debug, Deserialize, Clone)]
    pub struct Define {
        /// The preprocessor definition in the format `<name>[=<value>]`, e.g. `DEF` or `DEF=1`.
        pub define: String,
    }

    /// A include directory.
    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Include {
        /// The path to the include directory, represented with forward slashes.
        pub path: String,
        /// Whether the include directory is marked as a system include directory.
        #[serde(default)]
        pub is_system: bool,
    }

    /// Executable or shared library link information.
    #[derive(Debug, Deserialize, Clone)]
    #[serde(rename_all = "camelCase")]
    pub struct Link {
        ///  The language of the toolchain that is used to invoke the linker.
        pub language: Language,
        /// Fragments of the link command line invocation.
        pub command_fragments: Vec<CommandFragment>,
        /// Whether link-time optimization is enabled.
        #[serde(default)]
        pub lto: bool,
        /// Path to the sysroot.
        ///
        /// Present when the `CMAKE_SYSROOT_LINK` or `CMAKE_SYSROOT` variable is defined.
        pub sysroot: Option<Sysroot>,
    }

    /// A link command linke fragment.
    #[derive(Debug, Deserialize, Clone)]
    pub struct CommandFragment {
        /// A fragment of the link command line invocation.
        ///
        /// The value is encoded in the build system's native shell format.
        pub fragment: String,
        /// The role of the fragments content.
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

    /// Path to the sysroot.
    #[derive(Debug, Deserialize, Clone)]
    pub struct Sysroot {
        /// The absolute path to the sysroot, represented with forward slashes.
        pub path: PathBuf,
    }
}
