//! esp-idf source and tools installation.
//!
//! This module enables discovering and/or installing an `esp-idf` GIT repository,
//! and the corresponding tools for an `esp-idf` version.
//!
//! Right now, there are two locations where the `esp-idf` source and tools are
//! detected and installed:
//! - **[`install_dir`](Installer::install_dir)**
//!
//! - **`~/.espressif`**, if `install_dir` is None

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::{env, fs};

use anyhow::{anyhow, Context, Error, Result};
use serde::{Deserialize, Serialize};

use crate::python::PYTHON;
use crate::{cmd, git, path_buf, python};

use self::tools_schema::{
    PlatformDownloadInfo, PlatformOverrideInfoPlatformsItem, ToolInfo, VersionInfo,
};

#[cfg(feature = "elf")]
pub mod ulp_fsm;

mod tools_schema;

pub const DEFAULT_ESP_IDF_REPOSITORY: &str = "https://github.com/espressif/esp-idf.git";
pub const MANAGED_ESP_IDF_REPOS_DIR_BASE: &str = "esp-idf";

/// Environment variable containing the path to the esp-idf when in activated environment.
pub const IDF_PATH_VAR: &str = "IDF_PATH";
/// Environment variable containing the path to the tools required by the esp-idf.
pub const IDF_TOOLS_PATH_VAR: &str = "IDF_TOOLS_PATH";

const IDF_PYTHON_ENV_PATH_VAR: &str = "IDF_PYTHON_ENV_PATH";

/// The global install dir of the esp-idf and its tools, relative to the user home dir.
pub const GLOBAL_INSTALL_DIR: &str = ".espressif";

/// Default filename for the file that contains [`EspIdfBuildInfo`].
pub const BUILD_INFO_FILENAME: &str = "esp-idf-build.json";

/// One or more esp-idf tools.
#[derive(Debug, Clone)]
pub struct Tools {
    /// An optional path to the `tools.json` tools index to be used`.
    ///
    /// This file is passed to the `idf_tools.py` python script.
    pub index: Option<PathBuf>,
    /// All names of the tools that should be installed.
    pub tools: Vec<String>,
    _tempfile: Option<Arc<tempfile::TempPath>>,
}

impl Tools {
    /// Create a tools descriptor for tool names `tools` with the default tools index.
    pub fn new(tools: impl IntoIterator<Item = impl AsRef<str>>) -> Tools {
        Tools {
            index: None,
            tools: tools.into_iter().map(|s| s.as_ref().to_owned()).collect(),
            _tempfile: None,
        }
    }

    /// Create a tools descriptor for tool names `tools` with the path to the tools index
    /// `tools_json`.
    pub fn new_with_index(
        iter: impl IntoIterator<Item = impl AsRef<str>>,
        tools_json: impl AsRef<Path>,
    ) -> Tools {
        Tools {
            index: Some(tools_json.as_ref().into()),
            tools: iter.into_iter().map(|s| s.as_ref().to_owned()).collect(),
            _tempfile: None,
        }
    }

    /// Create a tools descriptor for tool names `tools` with the tools index containing
    /// `tools_json_content`.
    pub fn new_with_index_str(
        tools: Vec<String>,
        tools_json_content: impl AsRef<str>,
    ) -> Result<Tools> {
        let mut temp = tempfile::NamedTempFile::new()?;
        temp.as_file_mut()
            .write_all(tools_json_content.as_ref().as_bytes())?;
        let temp = temp.into_temp_path();

        Ok(Tools {
            index: Some(temp.to_path_buf()),
            tools,
            _tempfile: Some(Arc::new(temp)),
        })
    }

    /// Create a tools instance for installing cmake 3.20.3.
    pub fn cmake() -> Result<Tools> {
        Self::new_with_index_str(
            vec!["cmake".into()],
            include_str!("espidf/resources/cmake.json"),
        )
    }
}

/// A tool instance describing its properties.
#[derive(Debug, Default)]
struct Tool {
    name: String,
    /// url to obtain the Tool as an compressed binary
    url: String,
    /// version of the tool in no particular format
    version: String,
    /// hash of the compressed file
    sha256: String,
    /// size of the compressed file
    size: i64,
    /// Base absolute install dir as absolute Path
    install_dir: PathBuf,
    /// Path relative to install dir
    export_path: PathBuf,
    /// Command path and args that printout the current version of the tool
    /// - First element is the relative path to the command
    /// - Every other element represents a arg given to the cmd
    version_cmd_args: Vec<String>,
    /// regex to extract the version returned by the version_cmd
    version_regex: String,
}

impl Tool {
    /// Test if the tool is installed correctly
    fn test(&self) -> bool {
        let tool_path = self.abs_export_path();

        // if path does not exist -> tool is not installed
        if !tool_path.exists() {
            return false;
        }
        log::debug!(
            "Run cmd: {:?} to get current tool version",
            self.test_command(),
        );

        let output = self.test_command().output().unwrap_or_else(|e| {
            panic!(
                "Failed to run command: {:?}; error: {e:?}",
                self.test_command()
            )
        });

        let regex = regex::Regex::new(&self.version_regex).expect("Invalid regex pattern provided");

        if let Some(capture) = regex.captures(&String::from_utf8_lossy(&output.stdout)) {
            if let Some(var) = capture.get(0) {
                log::debug!("Match: {:?}, Version: {:?}", &var.as_str(), &self.version);
                return true;
            }
        }

        false
    }

    /// get the absolute PATH
    fn abs_export_path(&self) -> PathBuf {
        self.install_dir.join(self.export_path.as_path())
    }

    /// Creates a Command that will echo back the current version of the tool
    ///
    /// Since Command is non clonable this helper is provided
    fn test_command(&self) -> Command {
        let cmd_abs_path = self
            .abs_export_path()
            .join(self.version_cmd_args[0].clone());

        let mut version_cmd = std::process::Command::new(cmd_abs_path);
        version_cmd.args(self.version_cmd_args[1..].iter().cloned());
        version_cmd
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ToolsInfo {
    tools: Vec<ToolInfo>,
    version: u32,
}

fn parse_tools(
    tools_wanted: Vec<&str>,
    tools_json_file: PathBuf,
    install_dir: PathBuf,
) -> anyhow::Result<Vec<Tool>> {
    let mut tools_string = String::new();
    let mut tools_file = std::fs::File::open(tools_json_file)?;

    tools_file.read_to_string(&mut tools_string)?;

    let tools_info = serde_json::from_str::<ToolsInfo>(&tools_string)?;

    let tools = tools_info.tools;

    let tools = tools.iter().filter(|tool_info|{
        // tools_json schema contract marks name not as required ;(
        tools_wanted.contains(&tool_info.name.as_ref().unwrap().as_str())
    }).map(|tool_info| {
        let mut tool = Tool {
            name: tool_info.name.as_ref().unwrap().clone(),
            install_dir: install_dir.clone(),
            version_cmd_args: tool_info.version_cmd.to_vec(),
            version_regex: tool_info.version_regex.to_string(),
            ..Default::default()
        };

        tool_info.versions.iter().filter(|version| {
            version.status == Some(tools_schema::VersionInfoStatus::Recommended)
        }).for_each(|version| {

            let os_matcher = |info: &VersionInfo| -> Option<PlatformDownloadInfo> {
                let os = std::env::consts::OS;
                let arch = std::env::consts::ARCH;
                // The ARCH const in Rust does not differentiate between armel
                // and armhf. Assume armel for maximum compatibility.
                match (os, arch) {
                    ("linux", "x86") => info.linux_i686.clone(),
                    ("linux", "x86_64") => info.linux_amd64.clone(),
                    ("linux", "arm") => info.linux_armel.clone(),
                    ("linux", "aarch64") => info.linux_arm64.clone(),
                    ("macos", "x86_64") => info.macos.clone(),
                    ("macos", "aarch64") => info.macos_arm64.clone(),
                    ("windows", "x86") => info.win32.clone(),
                    ("windows", "x86_64") => info.win64.clone(),
                    _ => None,
                }
            };

            // either a any key is provided or only platform specific keys
            let info = if let Some(plaform_dll_info) = version.any.clone() {
                plaform_dll_info
            } else if let Some(plaform_dll_info) = os_matcher(version) {
                plaform_dll_info
            } else {
                panic!("Neither any or platform specifc match found. Please create an issue on https://github.com/esp-rs/embuild and report your operating system");
            };

            tool.url = info.url;
            tool.sha256 = info.sha256;
            tool.size = info.size;
            tool.version.clone_from(version.name.as_ref().unwrap());

            tool.export_path = PathBuf::new().join("tools").join(&tool.name).join(&tool.version);

            // export_path has two layers if indirection...
            // it seams only the first array is ever used
            let first_path = tool_info.export_paths.first();

            if let Some(path) = first_path {
                for element in path.iter() {
                    if !element.is_empty() {
                        tool.export_path = tool.export_path.join(element);
                    }
                }
            }
        });

        // Map OS and ARCH to platform names in esp-idf.
        // Unfortunately, the Rust std lib doesn't differentiate between armel
        // and armhf for 32-bit ARM platforms. This code defaults to armel for
        // maximum compatibility
        let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86") => Some(PlatformOverrideInfoPlatformsItem::LinuxI686),
            ("linux", "x86_64") => Some(PlatformOverrideInfoPlatformsItem::LinuxAmd64),
            ("linux", "arm") => Some(PlatformOverrideInfoPlatformsItem::LinuxArmel),
            ("linux", "aarch64") => Some(PlatformOverrideInfoPlatformsItem::LinuxArm64),
            ("macos", "x86_64") => Some(PlatformOverrideInfoPlatformsItem::Macos),
            ("macos", "aarch64") => Some(PlatformOverrideInfoPlatformsItem::MacosArm64),
            ("windows", "x86") => Some(PlatformOverrideInfoPlatformsItem::Win32),
            ("windows", "x86_64") => Some(PlatformOverrideInfoPlatformsItem::Win64),
            _ => None,
        };
        // Process any overrides that match the detected platform.
        // If additional fields from `tool_info` are used in the future, their
        // corresponding overrides need to be processed here as well
        if let Some(p) = platform {
            tool_info.platform_overrides
                .iter()
                .filter(|info| info.platforms.contains(&p))
                .for_each(|info| {
                    if let Some(export_path) = &info.export_paths {
                        // export_path can have multiple levels, but only the
                        // first is currently used in practice
                        if let Some(first_path) = export_path.first() {
                            tool.export_path = PathBuf::from_iter(
                                ["tools", &tool.name, &tool.version].into_iter()
                                .chain(first_path.iter().map(String::as_str))
                            );
                        }
                    }
                    if let Some(version_cmd) = &info.version_cmd {
                        tool.version_cmd_args = version_cmd.to_vec();
                    }
                    if let Some(version_regex) = &info.version_regex {
                        tool.version_regex = version_regex.to_string();
                    }
                });
        }

        log::debug!("{tool:?}");
        tool
    }
    ).collect();

    Ok(tools)
}

/// The error returned by [`EspIdf::try_from_env`].
#[derive(Debug, thiserror::Error)]
pub enum FromEnvError {
    /// No `esp-idf` repository detected in the environment.
    #[error("could not detect `esp-idf` repository in the environment")]
    NoRepo(#[source] anyhow::Error),
    /// An `esp-idf` repository exists but the environment is not activated.
    #[error("`esp-idf` repository exists but required tools not in environment")]
    NotActivated {
        /// The esp-idf repository detected from the environment.
        esp_idf_dir: SourceTree,
        /// The source error why detection failed.
        #[source]
        source: anyhow::Error,
    },
}

/// Information about a esp-idf source and tools installation.
#[derive(Debug)]
pub struct EspIdf {
    /// The esp-idf source tree.
    pub esp_idf_dir: SourceTree,
    /// The binary paths of all tools concatenated with the system `PATH` env variable.
    pub exported_path: OsString,
    /// The path to the python executable to be used by the esp-idf.
    pub venv_python: PathBuf,
    /// The version of the esp-idf or [`Err`] if it could not be detected.
    pub version: Result<EspIdfVersion>,
    /// Whether [`EspIdf::tree`] is a repository installed and managed by
    /// [`Installer`] and **not** provided by the user.
    pub is_managed_espidf: bool,
}

#[derive(Debug, Clone)]
pub enum SourceTree {
    Git(git::Repository),
    Plain(PathBuf),
}

impl SourceTree {
    pub fn open(path: &Path) -> Self {
        git::Repository::open(path)
            .map(SourceTree::Git)
            .unwrap_or_else(|_| SourceTree::Plain(path.to_owned()))
    }

    pub fn path(&self) -> &Path {
        match self {
            SourceTree::Git(repo) => repo.worktree(),
            SourceTree::Plain(path) => path,
        }
    }
}

impl EspIdf {
    /// Try to detect an activated esp-idf environment.
    pub fn try_from_env(idf_path: Option<&Path>) -> Result<EspIdf, FromEnvError> {
        let idf_path = idf_path.map(Path::to_owned).ok_or(()).or_else(|()| {
            // detect repo from $IDF_PATH if not passed by caller
            env::var_os(IDF_PATH_VAR)
                .map(|path| PathBuf::from(path))
                .ok_or_else(|| {
                    FromEnvError::NoRepo(anyhow!("environment variable `{IDF_PATH_VAR}` not found"))
                })
        })?;

        let esp_idf_dir = SourceTree::open(&idf_path);

        let path_var = env::var_os("PATH").unwrap_or_default();
        let not_activated = |source: Error| -> FromEnvError {
            FromEnvError::NotActivated {
                esp_idf_dir: esp_idf_dir.clone(),
                source,
            }
        };

        // get idf.py from $PATH
        // Special case for windows (see issue https://github.com/harryfei/which-rs/issues/56)
        let idf_py = if cfg!(windows) {
            env::split_paths(&path_var)
                .find_map(|p| {
                    let file_path = Path::new(&p).join("idf.py");
                    if file_path.is_file() {
                        Some(file_path)
                    } else {
                        None
                    }
                })
                .ok_or(which::Error::CannotFindBinaryPath)
        } else {
            which::which_in("idf.py", Some(&path_var), "")
        }
        .with_context(|| anyhow!("could not find `idf.py` in $PATH"))
        .map_err(not_activated)?;

        // make sure ${IDF_PATH}/tools/idf.py matches idf.py in $PATH
        let idf_py_repo = path_buf![esp_idf_dir.path(), "tools", "idf.py"];
        match (idf_py.canonicalize(), idf_py_repo.canonicalize()) {
            (Ok(a), Ok(b)) if a != b => {
                return Err(not_activated(anyhow!(
                    "missmatch between tools in $PATH ('{}') and esp-idf repository \
                         given by ${IDF_PATH_VAR} ('{}')",
                    a.display(),
                    b.display()
                )))
            }
            // ignore this check if canonicalize fails
            _ => (),
        };

        // get python from $PATH and make sure it has all required dependencies
        let python = which::which_in("python", Some(&path_var), "")
            .with_context(|| anyhow!("python not found in $PATH"))
            .map_err(not_activated)?;
        let check_python_deps_py =
            path_buf![esp_idf_dir.path(), "tools", "check_python_dependencies.py"];
        cmd!(&python, &check_python_deps_py)
            .stdout()
            .with_context(|| anyhow!("failed to check python dependencies"))
            .map_err(not_activated)?;

        Ok(EspIdf {
            version: EspIdfVersion::try_from(esp_idf_dir.path()),
            esp_idf_dir,
            exported_path: path_var,
            venv_python: python,
            is_managed_espidf: true,
        })
    }
}

/// The version of an esp-idf repository.
#[derive(Clone, Debug)]
pub struct EspIdfVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl EspIdfVersion {
    /// Try to extract the esp-idf version from an actual cloned repository.
    pub fn try_from(esp_idf_dir: &Path) -> Result<Self> {
        let version_cmake = path_buf![esp_idf_dir, "tools", "cmake", "version.cmake"];

        let base_err = || {
            anyhow!(
                "could not determine esp-idf version from '{}'",
                version_cmake.display()
            )
        };

        let s = fs::read_to_string(&version_cmake).with_context(base_err)?;
        let mut ver = [None; 3];
        s.lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix("set")?
                    .trim_start()
                    .strip_prefix('(')?
                    .strip_suffix(')')?
                    .split_once(' ')
            })
            .fold((), |_, (key, value)| {
                let index = match key.trim() {
                    "IDF_VERSION_MAJOR" => 0,
                    "IDF_VERSION_MINOR" => 1,
                    "IDF_VERSION_PATCH" => 2,
                    _ => return,
                };
                if let Ok(val) = value.trim().parse::<u64>() {
                    ver[index] = Some(val);
                }
            });
        if let [Some(major), Some(minor), Some(patch)] = ver {
            Ok(Self {
                major,
                minor,
                patch,
            })
        } else {
            Err(anyhow!("parsing failed").context(base_err()))
        }
    }

    /// Format an [`EspIdfVersion`] [`Result`] (e.g. from [`EspIdfVersion::try_from`]).
    pub fn format(ver: &Result<EspIdfVersion>) -> String {
        match ver {
            Ok(v) => format!("v{v}"),
            Err(_) => "(unknown version)".to_string(),
        }
    }
}

impl std::fmt::Display for EspIdfVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The origin of the esp-idf repository.
///
/// Two variations exist:
/// - Managed
///     The esp-idf source is installed automatically.
/// - Custom
///     A user-provided local clone the esp-idf repository.
///
/// In both cases the [`Installer`] will install all required tools.
///
/// The main difference between managed and custom esp-idf origin is reflected in their naming:
/// - [`EspIdfOrigin::Managed`] values are cloned locally by the [`Installer`] instance, inside its tooling installation directory.
///   Consenquently, these ESP-IDF repository clones will disappar if the installation directory is deleted by the user.
/// - [`EspIdfOrigin::Custom`] values are designating a user-provided, already cloned
///   ESP-IDF repository which lives outisde the [`Installer`]'s installation directory. It is
///   only read by the [`Installer`] so as to install the required tooling.
pub enum EspIdfOrigin {
    /// The [`Installer`] will install and manage the SDK.
    Managed(git::sdk::RemoteSdk),
    /// User-provided SDK repository untouched by the [`Installer`].
    Custom(SourceTree),
}

/// A distinct version of the esp-idf repository to be installed.
pub type EspIdfRemote = git::sdk::RemoteSdk;

/// Installer for the esp-idf source and tools.
pub struct Installer {
    esp_idf_origin: EspIdfOrigin,
    custom_install_dir: Option<PathBuf>,
    #[allow(clippy::type_complexity)]
    tools_provider:
        Option<Box<dyn FnOnce(&SourceTree, &Result<EspIdfVersion>) -> Result<Vec<Tools>>>>,
}

impl Installer {
    /// Create a installer using `esp_idf_origin`.
    pub fn new(esp_idf_origin: EspIdfOrigin) -> Installer {
        Self {
            esp_idf_origin,
            tools_provider: None,
            custom_install_dir: None,
        }
    }

    /// Add `tools` to the list of tools to install.
    #[must_use]
    pub fn with_tools<F>(mut self, provider: F) -> Self
    where
        F: 'static + FnOnce(&SourceTree, &Result<EspIdfVersion>) -> Result<Vec<Tools>>,
    {
        self.tools_provider = Some(Box::new(provider));
        self
    }

    /// Set the install dir to `install_dir`.
    ///
    /// If [`None`] use the default (see [`GLOBAL_INSTALL_DIR`]).
    #[must_use]
    pub fn install_dir(mut self, install_dir: Option<PathBuf>) -> Self {
        self.custom_install_dir = install_dir;
        self
    }

    /// Install the esp-idf source if a managed ESP-IDF reference was supplied by the user and then install all tools added with [`with_tools`](Self::with_tools).
    ///
    /// The install directory, where the esp-idf source and tools are installed into, is
    /// determined by
    /// 1. The directory given to [`install_dir`](Self::install_dir) if it is [`Some`],
    /// 2. or the global install directory `~/.espressif` (where `~` stands for the user
    ///    home directory) otherwise.
    ///
    /// Installation will do the following things in order:
    /// 1. If a [`EspIdfOrigin::Managed`] is provided, try to find an installed esp-idf
    ///    matching the specified remote repo. If not found, clone it into `<install
    ///    directory>/esp-idf[-<esp-idf-git-url-hash>]/<esp-idf version string>` where
    ///    `esp-idf version string` is the branch name, tag name, or the hash of the
    ///    commit, if a specific commit was used. Otherwise if it is a
    ///    [`EspIdfOrigin::Custom`] use that esp-idf repository instead.
    /// 2. Create a python virtual env using the system `python` and `idf_tools.py
    ///    install-python-env` in the install directory.
    /// 3. Install all tools with `idf_tools.py --tools-json <tools_json> install
    ///    <tools...>` per [`Tools`] instance added with [`with_tools`](Self::with_tools).
    ///    `tools_json` is the optional [`Tools::index`] path, if [`None`] the `tools.json`
    ///    of the esp-idf is used.
    pub fn install(self) -> Result<EspIdf> {
        let install_dir = self
            .custom_install_dir
            .unwrap_or_else(Self::global_install_dir);

        std::fs::create_dir_all(&install_dir).with_context(|| {
            format!(
                "could not create esp-idf install dir '{}'",
                install_dir.display()
            )
        })?;

        let (esp_idf_dir, managed_repo) = match self.esp_idf_origin {
            EspIdfOrigin::Managed(managed) => (
                SourceTree::Git(managed.open_or_clone(
                    &install_dir,
                    git::CloneOptions::new().depth(1),
                    DEFAULT_ESP_IDF_REPOSITORY,
                    MANAGED_ESP_IDF_REPOS_DIR_BASE,
                )?),
                true,
            ),
            EspIdfOrigin::Custom(tree) => (tree, false),
        };
        // Reading the version out of a cmake build file
        let esp_version = EspIdfVersion::try_from(&esp_idf_dir.path())?;
        let path_var_sep = if cfg!(windows) { ';' } else { ':' };

        // Create python virtualenv or use a previously installed one.

        // The systems minimal python version for bootstrepping the virtuelenv
        // - By "system python" we refer to the current python executable that is provided to this processs that is
        //   first found in the env PATH
        // - This will also be the python version used inside the virtualenv
        let python_version = python::check_python_at_least(3, 6)?;

        // Using the idf_tools.py script version that comes with the esp-idf git repository
        let idf_tools_py = path_buf![esp_idf_dir.path(), "tools", "idf_tools.py"];

        // TODO: add virtual_env check to skip install-python-env
        // running the command cost 2-3 seconds but always makes sure that everything is installed correctly and is up-to-date

        // assumes that the command can be run repeatedly
        // whenalready installed -> checks for updates and a working state
        cmd!(PYTHON, &idf_tools_py, "--idf-path", esp_idf_dir.path(), "--non-interactive", "install-python-env";
        env=(IDF_TOOLS_PATH_VAR, &install_dir), env_remove=("MSYSTEM"), env_remove=(IDF_PYTHON_ENV_PATH_VAR)).run()?;

        // since the above command exited sucessfully -> there should be a virt_env dir

        // the idf_tools.py templating name according to https://github.com/espressif/esp-idf/blob/master/tools/idf_tools.py#L99
        // uses always the systems python version -> idf{ESP_IDF_MAJOR_MINOR_VERSION}_py{SYSTEM_PYTHON_MAJOR_MINOR}_env,

        // with above knowladge -> construct the python_env_dir implicitly
        let idf_major_minor = format!("{}.{}", esp_version.major, esp_version.minor);
        let python_major_minor = format!("{}.{}", python_version.major, python_version.minor);

        let python_env_dir_template = format!("idf{idf_major_minor}_py{python_major_minor}_env");

        let python_env_dir = path_buf![&install_dir, "python_env", python_env_dir_template];

        let esp_version = Ok(esp_version);

        #[cfg(windows)]
        let venv_python = PathBuf::from(python_env_dir).join("Scripts/python");

        #[cfg(not(windows))]
        let venv_python = python_env_dir.join("bin/python");

        log::debug!("Start installing tools");

        // End: Install virt_env
        // Section: Install tools.

        let tools = self
            .tools_provider
            .map(|p| p(&esp_idf_dir, &esp_version))
            .unwrap_or(Ok(Vec::new()))?;

        let tools_wanted = tools.clone();
        let tools_wanted: Vec<&str> = tools_wanted
            .iter()
            .flat_map(|tool| tool.tools.iter().map(|s| s.as_str()))
            .collect();

        let tools_json = esp_idf_dir.path().join("tools/tools.json");

        let tools_vec = parse_tools(
            tools_wanted.clone(),
            tools_json.clone(),
            install_dir.clone(),
        )
        .unwrap();

        //let tools_vec = parse_into_tools(tools_wanted, tools_json, install_dir.clone())?;

        let all_tools_installed = tools_vec.iter().all(|tool| tool.test());

        if !all_tools_installed {
            for tool_set in tools {
                let tools_json = tool_set
                    .index
                    .as_ref()
                    .map(|tools_json| {
                        [OsStr::new("--tools-json"), tools_json.as_os_str()].into_iter()
                    })
                    .into_iter()
                    .flatten();

                cmd!(&venv_python, &idf_tools_py, "--idf-path", esp_idf_dir.path(), @tools_json.clone(), "install"; 
                     env=(IDF_TOOLS_PATH_VAR, &install_dir), args=(tool_set.tools)).run()?;
            }

            // Test again if all tools are now installed correctly
            let all_tools_installed = tools_vec.iter().all(|tool| tool.test());
            if !all_tools_installed {
                return Err(anyhow::Error::msg("Could not install all requested Tools"));
            }
        }

        // End Tools install
        // Create PATH

        // All tools are installed -> infer there PATH variable by using the information out of tools.json
        let mut tools_path: Vec<PathBuf> = tools_vec
            .iter()
            .map(|tool| tool.abs_export_path())
            .collect();

        // add the python virtual env to the export path
        let mut python_path = venv_python.clone();
        python_path.pop();
        tools_path.push(python_path);

        let paths = env::join_paths(
            tools_path
                .into_iter()
                .map(PathBuf::from)
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )?;

        log::debug!("Using PATH='{}'", &paths.to_string_lossy());

        Ok(EspIdf {
            esp_idf_dir,
            exported_path: paths,
            venv_python,
            version: esp_version,
            is_managed_espidf: managed_repo,
        })
    }

    /// Get the global install dir.
    ///
    /// Panics if the OS does not provide a home directory.
    pub fn global_install_dir() -> PathBuf {
        home::home_dir()
            .expect("No home directory available for this operating system")
            .join(GLOBAL_INSTALL_DIR)
    }
}

/// Parse a [`git::Ref`] from an esp-idf version string.
///
/// The version string can have the following format:
/// - `commit:<hash>`: Uses the commit `<hash>` of the `esp-idf` repository. Note that
///                    this will clone the whole `esp-idf` not just one commit.
/// - `tag:<tag>`: Uses the tag `<tag>` of the `esp-idf` repository.
/// - `branch:<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
/// - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the `esp-idf` repository.
/// - `<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
pub fn parse_esp_idf_git_ref(version: &str) -> git::Ref {
    git::Ref::parse(version)
}

/// Info about the esp-idf build.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct EspIdfBuildInfo {
    /// The directory of the local cloned esp-idf repository that was used for the build.
    pub esp_idf_dir: PathBuf,
    /// The exported PATH environment variable containing all tools.
    pub exported_path_var: String,
    /// Path to the python executable in the esp-idf virtual environment.
    pub venv_python: PathBuf,
    /// CMake build dir containing all build artifacts.
    pub build_dir: PathBuf,
    /// CMake project dir containing the dummy cmake project.
    pub project_dir: PathBuf,
    /// Compiler path used to compile the esp-idf.
    pub compiler: PathBuf,
    /// MCU name that esp-idf was compiled for.
    pub mcu: String,
    /// sdkconfig file used to configure the esp-idf.
    pub sdkconfig: Option<PathBuf>,
    /// All sdkconfig defaults files used for the build.
    pub sdkconfig_defaults: Option<Vec<PathBuf>>,
}

impl EspIdfBuildInfo {
    /// Deserialize from the given JSON file.
    pub fn from_json(path: impl AsRef<Path>) -> Result<EspIdfBuildInfo> {
        let file = std::fs::File::open(&path)
            .with_context(|| anyhow!("Could not read {}", path.as_ref().display()))?;
        let result: EspIdfBuildInfo = serde_json::from_reader(file)?;
        Ok(result)
    }

    /// Save as a JSON file at `path`.
    pub fn save_json(&self, path: impl AsRef<Path>) -> Result<()> {
        let file = std::fs::File::create(&path)
            .with_context(|| anyhow!("Could not write {}", path.as_ref().display()))?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

/// This module is a bit of a hack as it contains special support for the `esp-idf-sys`, `esp-idf-hal` and `esp-idf-svc` crates
/// (So in a way the `embuild` library now knows about the existence of those.)
///
/// Yet - and for any binary crate that depends on ANY of the above crates -
/// it enables easy access to the hidden ESP IDF build that these crates do -
/// as in link args, kconfig (including as Rust `#[cfg()]` directives), include dirs, path etc.
///
/// For example, to have your binary crate link against ESP IDF,
/// and also to be able to consume - as `#[cfg()]` -  the ESP IDF configuration settings,
/// just create a `build.rs` file in your binary crate that contains the following one-liner:
/// ```ignore
/// fn main() {
///     embuild::espidf::sysenv::output();
/// }
/// ```
pub mod sysenv {
    use std::env;

    use crate::{
        build::{CInclArgs, CfgArgs, LinkArgs},
        cargo,
    };

    const CRATES_LINKS_LIBS: [&str; 3] = ["ESP_IDF_SVC", "ESP_IDF_HAL", "ESP_IDF"];

    pub fn cfg_args() -> Option<CfgArgs> {
        CRATES_LINKS_LIBS
            .iter()
            .filter_map(|lib| CfgArgs::try_from_env(lib).ok())
            .next()
    }

    pub fn cincl_args() -> Option<CInclArgs> {
        CRATES_LINKS_LIBS
            .iter()
            .filter_map(|lib| CInclArgs::try_from_env(lib).ok())
            .next()
    }

    pub fn link_args() -> Option<LinkArgs> {
        CRATES_LINKS_LIBS
            .iter()
            .filter_map(|lib| LinkArgs::try_from_env(lib).ok())
            .next()
    }

    pub fn env_path() -> Option<String> {
        CRATES_LINKS_LIBS
            .iter()
            .filter_map(|lib| env::var(format!("DEP_{lib}_{}", crate::build::ENV_PATH_VAR)).ok())
            .next()
    }

    pub fn idf_path() -> Option<String> {
        CRATES_LINKS_LIBS
            .iter()
            .filter_map(|lib| {
                env::var(format!("DEP_{lib}_{}", crate::build::ESP_IDF_PATH_VAR)).ok()
            })
            .next()
    }

    /// For internal use by the `esp-idf-*` crates only
    pub fn relay() {
        if let Some(args) = cfg_args() {
            args.propagate()
        }
        if let Some(args) = cincl_args() {
            args.propagate()
        }
        if let Some(args) = link_args() {
            args.propagate()
        }
        if let Some(path) = env_path() {
            cargo::set_metadata(crate::build::ENV_PATH_VAR, path)
        }
        if let Some(path) = idf_path() {
            cargo::set_metadata(crate::build::ESP_IDF_PATH_VAR, path)
        }
    }

    pub fn output() {
        if let Some(args) = cfg_args() {
            args.output()
        }
        if let Some(args) = link_args() {
            args.output()
        }
    }
}
