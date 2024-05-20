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

use crate::python::PYTHON;
use crate::{cmd, git, path_buf, python};

#[cfg(feature = "elf")]
pub mod ulp_fsm;

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
#[derive(Debug)]
struct Tool {
    name: String,
    /// url to obtain the Tool as an compressed binary
    url: String,
    /// version of the tool in no particular format
    versions: String,
    /// hash of the compressed file
    sha256: String,
    /// size of the compressed file
    size: u64,
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

        let output = self
            .test_command()
            .output()
            .unwrap_or_else(|_| panic!("Failed to run command: {:?}", self.test_command()));

        let regex = regex::Regex::new(&self.version_regex).expect("Invalid regex pattern provided");

        if let Some(capture) = regex.captures(&String::from_utf8_lossy(&output.stdout)) {
            if let Some(var) = capture.get(0) {
                log::debug!("Match: {:?}, Version: {:?}", &var.as_str(), &self.versions);
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

/// Parsing a provided tools.json file, and return a Vec<Tool> representing a Tool version of the wanted tools
fn parse_into_tools(
    tools_wanted: Vec<&str>,
    tools_json_file: PathBuf,
    install_dir: PathBuf,
) -> anyhow::Result<Vec<Tool>> {
    let mut tools: Vec<Tool> = Vec::new();

    let os_key = get_os_target_key().unwrap();

    let mut tools_string = String::new();
    let mut tools_file = std::fs::File::open(tools_json_file)?;

    tools_file.read_to_string(&mut tools_string)?;

    let parsed_file = serde_json::from_str::<serde_json::Value>(&tools_string)?;
    let tools_object = parsed_file["tools"]
        .as_array()
        .expect("JSON-PARSING-ERROR: make sure the provided tools.json in the esp-idf repository is not malformed");

    for tool_object in tools_object.iter().filter(|parsed_tool| {
        tools_wanted.contains(
            &parsed_tool["name"]
                .as_str()
                .expect("JSON-PARSING-ERROR: make sure the provided tools.json in the esp-idf repository is not malformed"),
        )
    }) {
        let name = tool_object["name"].as_str().unwrap();

        log::debug!("============================================================================");
        log::debug!("Tool name: {name}");

        // notice that export_paths inside the tools.json has the structure of "key: [ [ "path1", "path2", ... ] ,]"
        // -> to layers of indirection to get the actual path
        // it seams only the first array is ever used
        let export_path = tool_object["export_paths"][0].clone();
        let export_path: Vec<&str> = export_path
            .as_array()
            .and_then(|path_array| path_array.iter().map(|path| path.as_str()).collect())
            .unwrap_or_else(Vec::new);

        log::debug!("export_path: {export_path:?}");

        let version_cmd_args = tool_object["version_cmd"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<String>>();

        let version_regex = tool_object["version_regex"].as_str().unwrap().to_string();

        let mut tool = Tool {
            name: name.to_string(),
            url: String::new(),
            versions: String::new(),
            sha256: String::new(),
            size: 0,
            install_dir: install_dir.clone(),
            export_path: PathBuf::new(),
            version_cmd_args,
            version_regex,
        };

        // pick the recommended version out of a list of versions for the current os
        tool_object
            .get("versions")
            .and_then(|versions| versions.as_array())
            .unwrap()
            .iter()
            .filter(|version| {
                // filter by version object where key "status" is "recommended"
                let inner = version.as_object().unwrap();
                inner.get("status").unwrap().as_str() == Some("recommended")
            })
            .for_each(|version| {
                // only insert the version object if it contains the correct os key
                let inner = version.as_object().unwrap();
                if let Some(os_version) = inner.get(os_key) {
                    if let Some(url) = os_version.get("url") { tool.url = url.as_str().unwrap().to_string(); }
                    if let Some(sha256) = os_version.get("sha256") { tool.sha256 = sha256.as_str().unwrap().to_string(); }
                    if let Some(size) = os_version.get("size") { tool.size = size.as_u64().unwrap(); }
                    if let Some(name) = version.get("name") { tool.versions = name.as_str().unwrap().to_string(); }

                    tool.export_path = PathBuf::new()
                        .join("tools")
                        .join(&tool.name)
                        .join(&tool.versions);
                    for p in export_path.iter() {
                        tool.export_path = tool.export_path.join(p);
                    }
                }
            });

        tools.push(tool);
    }
    log::debug!("============================================================================");

    Ok(tools)
}

// Maps the current os and architecture to the correct key in the tools.json file
fn get_os_target_key() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    
    match os {
        "linux" => match arch {
            "x86_64" => Some("linux-amd64"),
            // TODO add and test arm variants
            _ => None,
        },
        "windows" => match arch {
            "x86" => Some("win32"),
            "x86_64" => Some("win64"),
            _ => None,
        },
        "macos" => match arch {
            "aarch64" => Some("macos-arm64"),
            "x86_64" => Some("macos"),
            _ => None,
        },
        _ => None,
    }
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
        esp_idf_repo: git::Repository,
        /// The source error why detection failed.
        #[source]
        source: anyhow::Error,
    },
}

/// Information about a esp-idf source and tools installation.
#[derive(Debug)]
pub struct EspIdf {
    /// The esp-idf repository.
    pub repository: git::Repository,
    /// The binary paths of all tools concatenated with the system `PATH` env variable.
    pub exported_path: OsString,
    /// The path to the python executable to be used by the esp-idf.
    pub venv_python: PathBuf,
    /// The version of the esp-idf or [`Err`] if it could not be detected.
    pub version: Result<EspIdfVersion>,
    /// Whether [`EspIdf::repository`] is installed and managed by [`Installer`] and
    /// **not** provided by the user.
    pub is_managed_espidf: bool,
}

impl EspIdf {
    /// Try to detect an activated esp-idf environment.
    pub fn try_from_env() -> Result<EspIdf, FromEnvError> {
        // detect repo from $IDF_PATH
        let idf_path = env::var_os(IDF_PATH_VAR).ok_or_else(|| {
            FromEnvError::NoRepo(anyhow!("environment variable `{IDF_PATH_VAR}` not found"))
        })?;
        let repo = git::Repository::open(idf_path).map_err(FromEnvError::NoRepo)?;

        let path_var = env::var_os("PATH").unwrap_or_default();
        let not_activated = |source: Error| -> FromEnvError {
            FromEnvError::NotActivated {
                esp_idf_repo: repo.clone(),
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
        let idf_py_repo = path_buf![repo.worktree(), "tools", "idf.py"];
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
            path_buf![repo.worktree(), "tools", "check_python_dependencies.py"];
        cmd!(&python, &check_python_deps_py)
            .stdout()
            .with_context(|| anyhow!("failed to check python dependencies"))
            .map_err(not_activated)?;

        Ok(EspIdf {
            version: EspIdfVersion::try_from(&repo),
            repository: repo,
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
    pub fn try_from(repo: &git::Repository) -> Result<Self> {
        let version_cmake = path_buf![repo.worktree(), "tools", "cmake", "version.cmake"];

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
pub type EspIdfOrigin = git::sdk::SdkOrigin;

/// A distinct version of the esp-idf repository to be installed.
pub type EspIdfRemote = git::sdk::RemoteSdk;

/// Installer for the esp-idf source and tools.
pub struct Installer {
    esp_idf_origin: EspIdfOrigin,
    custom_install_dir: Option<PathBuf>,
    #[allow(clippy::type_complexity)]
    tools_provider:
        Option<Box<dyn FnOnce(&git::Repository, &Result<EspIdfVersion>) -> Result<Vec<Tools>>>>,
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
        F: 'static + FnOnce(&git::Repository, &Result<EspIdfVersion>) -> Result<Vec<Tools>>,
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

        let (repository, managed_repo) = match self.esp_idf_origin {
            EspIdfOrigin::Managed(managed) => (
                managed.open_or_clone(
                    &install_dir,
                    git::CloneOptions::new().depth(1),
                    DEFAULT_ESP_IDF_REPOSITORY,
                    MANAGED_ESP_IDF_REPOS_DIR_BASE,
                )?,
                true,
            ),
            EspIdfOrigin::Custom(repository) => (repository, false),
        };

        // Reading the version out of a cmake build file
        let esp_version = EspIdfVersion::try_from(&repository)?;

        // Create python virtualenv or use a previously installed one.

        // The systems minimal python version for bootstrepping the virtuelenv
        // - By "system python" we refer to the current python executable that is provided to this processs that is
        //   first found in the env PATH
        // - This will also be the python version used inside the virtualenv
        let python_version = python::check_python_at_least(3, 6)?;

        // Using the idf_tools.py script version that comes with the esp-idf git repository
        let idf_tools_py = path_buf![repository.worktree(), "tools", "idf_tools.py"];

        // TODO: add virtual_env check to skip install-python-env
        // running the command cost 2-3 seconds but always makes sure that everything is installed correctly and is up-to-date

        // assumes that the command can be run repeatedly
        // whenalready installed -> checks for updates and a working state
        cmd!(PYTHON, &idf_tools_py, "--idf-path", repository.worktree(), "--non-interactive", "install-python-env";
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
            .map(|p| p(&repository, &esp_version))
            .unwrap_or(Ok(Vec::new()))?;

        let tools_wanted = tools.clone();
        let tools_wanted: Vec<&str> = tools_wanted
            .iter()
            .flat_map(|tool| tool.tools.iter().map(|s| s.as_str()))
            .collect();

        let tools_json = repository.worktree().join("tools/tools.json");

        let tools_vec = parse_into_tools(tools_wanted, tools_json, install_dir.clone())?;

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

                cmd!(&venv_python, &idf_tools_py, "--idf-path", repository.worktree(), @tools_json.clone(), "install"; 
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
            repository,
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
