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

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};

use crate::python::PYTHON;
use crate::{cmd, cmd_output, git, path_buf, python};

pub const IDF_PATH_VAR: &str = "IDF_PATH";

const DEFAULT_ESP_IDF_REPOSITORY: &str = "https://github.com/espressif/esp-idf.git";
const MANAGED_ESP_IDF_REPOS_DIR: &str = "esp-idf";

/// The global install dir of the esp-idf and its tools, relative to the user home dir.
pub const GLOBAL_INSTALL_DIR: &str = ".espressif";

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

/// Information about a esp-idf source and tools installation.
pub struct EspIdf {
    /// The esp-idf repository.
    pub repository: git::Repository,
    /// The binary paths of all tools concatenated with the system `PATH` env variable.
    pub exported_path: OsString,
}

impl EspIdf {
    pub fn try_from_env() -> Result<Option<Self>> {
        if cmd!["python", "idf.py", "--version"].is_ok() {
            let repository = match env::var(IDF_PATH_VAR) {
                Err(env::VarError::NotPresent) => anyhow::bail!(
                    "Active ESP-IDf environment detected, however variable {} is not set",
                    IDF_PATH_VAR
                ),
                r => git::Repository::open(r?)?,
            };

            let idf = Self {
                repository,
                exported_path: env::var_os("PATH").unwrap_or_else(OsString::new),
            };

            Ok(Some(idf))
        } else {
            Ok(None)
        }
    }
}

/// EspIdfReference can be either a managed or unmanaged one.
///
/// The managed one is represented by a GIT url and a git Ref. The URL is optional, and if not provided
/// will default to the main ESP-IDF GitHub repository.
///
/// An unmanaged ESP-IDF reference is represented by a user-provided local clone of ESP-IDF.
///
/// The main difference between managed and unmanaged ESP-IDF references is reflected in their naming:
/// - EspIdfReference::Managed values are cloned locally by the Installer instance, inside its tooling installation directory.
///   Consenquently, these ESP-IDF repository clones will disappar if the installation directory is deleted by the user.
/// - EspIdfReferencen::Unmanaged values are designating a user-provided, already cloned ESP-IDF repository which lives
///   outisde the Installer's installation directory. It is only read by the Installer so as the required tooling
///   for this ESP-IDF repository to be installed.
#[derive(Debug, Clone)]
pub enum EspIdfReference {
    Managed(Managed),
    Unmanaged(git::Repository),
}

/// Managed is a data structure that keeps the (optional) GIT url and the GIT ref for a managed EspIdfReference instance.
#[derive(Debug, Clone)]
pub struct Managed {
    pub repo_url: Option<String>,
    pub git_ref: git::Ref,
}

impl Managed {
    /// Decode a [`git::Ref`] from an esp-idf version string.
    ///
    /// The version string can have the following format:
    /// - `commit:<hash>`: Uses the commit `<hash>` of the `esp-idf` repository. Note that
    ///                    this will clone the whole `esp-idf` not just one commit.
    /// - `tag:<tag>`: Uses the tag `<tag>` of the `esp-idf` repository.
    /// - `branch:<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
    /// - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the `esp-idf` repository.
    /// - `<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
    pub fn decode_version_ref(version: &str) -> git::Ref {
        let version = version.trim();
        assert!(
            !version.is_empty(),
            "esp-idf version ('{}') must be non-empty",
            version
        );

        match version.split_once(':') {
            Some(("commit", c)) => git::Ref::Commit(c.to_owned()),
            Some(("tag", t)) => git::Ref::Tag(t.to_owned()),
            Some(("branch", b)) => git::Ref::Branch(b.to_owned()),
            _ => match version.chars().next() {
                Some(c) if c.is_ascii_digit() => git::Ref::Tag("v".to_owned() + version),
                Some('v')
                    if version.len() > 1 && version.chars().nth(1).unwrap().is_ascii_digit() =>
                {
                    git::Ref::Tag(version.to_owned())
                }
                Some(_) => git::Ref::Branch(version.to_owned()),
                _ => unreachable!(),
            },
        }
    }

    /// Return the URL of the GIT repository.
    /// If `repo_url` is None, then the default ESP-IDf repository is returned.
    pub fn repo_url(&self) -> &str {
        self.repo_url
            .as_deref()
            .unwrap_or(DEFAULT_ESP_IDF_REPOSITORY)
    }

    fn resolve(&self, repos_dir: &Path) -> Result<git::Repository> {
        let repo_path = repos_dir
            .join(self.url_hash())
            .join(self.git_ref.as_string());

        let repository = if repo_path.exists() {
            let repository = git::Repository::open(&repo_path)?;

            if !repository.is_ref(&self.git_ref) {
                anyhow::bail!(
                    "Repository {} is not matching GIT ref {}",
                    repository.worktree().display(),
                    self.git_ref
                );
            }

            repository
        } else {
            let mut repository = git::Repository::new(&repo_path);

            repository.clone_ext(
                self.repo_url(),
                git::CloneOptions::new()
                    .force_ref(self.git_ref.clone())
                    .depth(1),
            )?;

            repository
        };

        Ok(repository)
    }

    fn url_hash(&self) -> String {
        let mut hasher = DefaultHasher::new();

        self.repo_url().hash(&mut hasher);

        format!("{:x}", hasher.finish())
    }
}

/// Installer for the esp-idf source and tools.
#[derive(Debug, Clone)]
pub struct Installer {
    esp_idf_reference: EspIdfReference,
    install_dir: Option<PathBuf>,
    tools: Vec<Tools>,
}

impl Installer {
    /// Create a new installer for the `esp_idf_reference`.
    pub fn new(esp_idf_reference: EspIdfReference) -> Installer {
        Self {
            esp_idf_reference,
            tools: vec![],
            install_dir: None,
        }
    }

    /// Add `tools` to the list of tools to install.
    #[must_use]
    pub fn with_tools(mut self, tools: Tools) -> Self {
        self.tools.push(tools);
        self
    }

    /// Set the install dir to `install_dir`. If [`None`] use the default.
    #[must_use]
    pub fn install_dir(mut self, install_dir: Option<PathBuf>) -> Self {
        self.install_dir = install_dir;
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
    /// 1. If a remote ESP-IDF reference is provided, try to find an installed esp-idf matching the
    ///    specified reference.
    ///    - If not found, clone it into `<install directory>/esp-idf/<esp-idf-git-url-hash>/<esp-idf version string>` where
    ///      `esp-idf version string` is the branch name, tag name, or the hash of the commit, if a specific commit was used.
    /// 2. Create a python virtual env using the system `python` and `idf_tools.py
    ///   install-python-env` in the install directory.
    /// 3. Install all tools with `idf_tools.py --tools-json <tools_json> install <tools...>`
    ///   per [`Tools`] instance added with [`with_tools`](Self::with_tools). `tools_json`
    ///   is the optional [`Tools::index`] path, if [`None`] the `tools.json` of the
    ///   esp-idf is used.
    pub fn install(self) -> Result<EspIdf> {
        let install_dir = self.install_dir.map(Result::Ok).unwrap_or_else(|| {
            Self::global_install_dir().ok_or_else(|| anyhow!("No system home directory"))
        })?;

        std::fs::create_dir_all(&install_dir).with_context(|| {
            format!(
                "Could not create esp-idf install dir '{}'",
                install_dir.display()
            )
        })?;

        let repository = match self.esp_idf_reference {
            EspIdfReference::Managed(managed) => {
                managed.resolve(&install_dir.join(MANAGED_ESP_IDF_REPOS_DIR))?
            }
            EspIdfReference::Unmanaged(repository) => repository,
        };

        let path_var_sep = if cfg!(not(windows)) { ':' } else { ';' };

        // Create python virtualenv or use a previously installed one.

        // TODO: also install python
        python::check_python_at_least(3, 6)?;

        let idf_tools_py = path_buf![repository.worktree(), "tools", "idf_tools.py"];

        let get_python_env_dir = || -> Result<String> {
            Ok(cmd_output!(PYTHON, &idf_tools_py, "--idf-path", repository.worktree(), "--quiet", "export", "--format=key-value";
                       ignore_exitcode, env=("IDF_TOOLS_PATH", &install_dir), env_remove=("MSYSTEM"))?
                            .lines()
                            .find(|s| s.trim_start().starts_with("IDF_PYTHON_ENV_PATH="))
                            .ok_or_else(|| anyhow!("`idf_tools.py export` result contains no `IDF_PYTHON_ENV_PATH` item"))?
                            .trim()
                            .strip_prefix("IDF_PYTHON_ENV_PATH=").unwrap()
                                  .to_string())
        };

        let python_env_dir: PathBuf = match get_python_env_dir() {
            Ok(dir) if Path::new(&dir).exists() => dir,
            _ => {
                cmd!(PYTHON, &idf_tools_py, "--idf-path", repository.worktree(), "--quiet", "--non-interactive", "install-python-env";
                     env=("IDF_TOOLS_PATH", &install_dir))?;
                get_python_env_dir()?
            }
        }.into();

        // TODO: better way to get the virtualenv python executable
        let python = which::which_in(
            "python",
            #[cfg(windows)]
            Some(&python_env_dir.join("Scripts")),
            #[cfg(not(windows))]
            Some(&python_env_dir.join("bin")),
            std::env::current_dir()?,
        )?;

        // Install tools.
        let mut exported_paths = HashSet::new();
        for tool in self.tools {
            let tools_json = tool
                .index
                .as_ref()
                .map(|tools_json| {
                    std::iter::IntoIterator::into_iter([
                        OsStr::new("--tools-json"),
                        tools_json.as_os_str(),
                    ])
                })
                .into_iter()
                .flatten();

            // Install the tools.
            cmd!(python, &idf_tools_py, "--idf-path", repository.worktree(), @tools_json.clone(), "install"; 
                 env=("IDF_TOOLS_PATH", &install_dir), args=(tool.tools))?;

            // Get the paths to the tools.
            //
            // Note: `idf_tools.py` queries the environment
            // variable `MSYSTEM` to determine if it should convert the paths to its shell
            // equivalent on windows
            // (https://github.com/espressif/esp-idf/blob/bcbef9a8db54d2deef83402f6e4403ccf298803a/tools/idf_tools.py#L243)
            // (for example to unix paths when using msys or cygwin), but we need Windows
            // native paths in rust. So we remove that environment variable when calling
            // idf_tools.py.
            exported_paths.extend(
                cmd_output!(python, &idf_tools_py, "--idf-path", repository.worktree(), @tools_json, "--quiet", "export", "--format=key-value";
                                ignore_exitcode, env=("IDF_TOOLS_PATH", &install_dir), env_remove=("MSYSTEM"))?
                            .lines()
                            .find(|s| s.trim_start().starts_with("PATH="))
                            .expect("`idf_tools.py export` result contains no `PATH` item").trim()
                            .strip_prefix("PATH=").unwrap()
                            .rsplit_once(path_var_sep).unwrap().0 // remove $PATH, %PATH%
                            .split(path_var_sep)
                            .map(|s| s.to_owned())
            );
        }

        let paths = env::join_paths(
            exported_paths
                .into_iter()
                .map(PathBuf::from)
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )?;

        log::debug!("Using PATH='{}'", &paths.to_string_lossy());

        Ok(EspIdf {
            repository,
            exported_path: paths,
        })
    }

    fn global_install_dir() -> Option<PathBuf> {
        Some(dirs::home_dir()?.join(GLOBAL_INSTALL_DIR))
    }
}

/// Info about the esp-idf build.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct EspIdfBuildInfo {
    /// The install dir where all tools for the compilation of the esp-idf are installed.
    pub install_dir: PathBuf,
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
