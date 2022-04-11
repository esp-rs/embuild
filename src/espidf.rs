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
use std::ffi::{OsStr, OsString};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{env, fs};

use anyhow::{anyhow, Context, Error, Result};

use crate::python::PYTHON;
use crate::{cmd, cmd_output, git, path_buf, python};

pub mod ulp_fsm;

const DEFAULT_ESP_IDF_REPOSITORY: &str = "https://github.com/espressif/esp-idf.git";
const MANAGED_ESP_IDF_REPOS_DIR_BASE: &str = "esp-idf";

/// Environment variable containing the path to the esp-idf when in activated environment.
pub const IDF_PATH_VAR: &str = "IDF_PATH";

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
        let repo = git::Repository::open(&idf_path).map_err(FromEnvError::NoRepo)?;

        let path_var = env::var_os("PATH").unwrap_or_default();
        let not_activated = |source: Error| -> FromEnvError {
            FromEnvError::NotActivated {
                esp_idf_repo: repo.clone(),
                source,
            }
        };

        // get idf.py from $PATH
        let idf_py = which::which_in("idf.py", Some(&path_var), "")
            .with_context(|| anyhow!("could not find `idf.py` in $PATH"))
            .map_err(not_activated)?;

        // make sure ${IDF_PATH}/tools/idf.py matches idf.py in $PATH
        let idf_py_repo = path_buf![repo.worktree(), "tools", "idf.py"];
        match (idf_py.canonicalize(), idf_py_repo.canonicalize()) {
            (Ok(a), Ok(b)) if a != b => {
                return Err(not_activated(
                    anyhow!(
                        "missmatch between tools in $PATH ('{}') and esp-idf repository given by $IDF_PATH ('{}')",
                        a.display(), b.display()
                    ),
                ))
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
        cmd_output!(&python, &check_python_deps_py)
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
#[derive(Debug, Clone)]
pub enum EspIdfOrigin {
    /// The [`Installer`] will install and manage the esp-idf.
    Managed(EspIdfRemote),
    /// User-provided esp-idf repository untouched by the [`Installer`].
    Custom(git::Repository),
}

/// A distinct version of the esp-idf repository to be installed.
#[derive(Debug, Clone)]
pub struct EspIdfRemote {
    /// Optional custom URL to the git repository.
    pub repo_url: Option<String>,
    /// A [`git::Ref`] for the commit, tag or branch to be used.
    pub git_ref: git::Ref,
}

impl EspIdfRemote {
    /// Return the URL of the GIT repository.
    /// If `repo_url` is [`None`], then the default ESP-IDf repository is returned.
    pub fn repo_url(&self) -> &str {
        self.repo_url
            .as_deref()
            .unwrap_or(DEFAULT_ESP_IDF_REPOSITORY)
    }

    /// Clone the repository or open if it exists and matches [`EspIdfRemote::git_ref`].
    fn open_or_clone(&self, install_dir: &Path) -> Result<git::Repository> {
        // Only append a hash of the git remote URL to the parent folder name of the
        // repository if this is not the default remote.
        let folder_name = if let Some(hash) = self.url_hash() {
            format!("{MANAGED_ESP_IDF_REPOS_DIR_BASE}-{hash}")
        } else {
            MANAGED_ESP_IDF_REPOS_DIR_BASE.to_owned()
        };
        let repos_dir = install_dir.join(folder_name);
        if !repos_dir.exists() {
            fs::create_dir(&repos_dir)
                .with_context(|| anyhow!("could not create folder '{}'", repos_dir.display()))?;
        }

        let repo_path = repos_dir.join(self.repo_dir());
        let mut repository = git::Repository::new(&repo_path);

        repository.clone_ext(
            self.repo_url(),
            git::CloneOptions::new()
                .force_ref(self.git_ref.clone())
                .depth(1),
        )?;

        Ok(repository)
    }

    /// Create a hash when a custom repo_url is specified.
    fn url_hash(&self) -> Option<String> {
        // This uses the default hasher from the standard library, which is not guaranteed
        // to be the same across versions, but if the hash algorithm changes and assuming
        // a different hash, the logic above will happily clone the repo in a different
        // directory. It also uses a 64 bit hash by which the chance for collisions is
        // pretty small (assuming a good hash function) and even if there is a collision
        // it will still work (and also even if the ref is the same), though the cloned
        // repo will be in the same folder as a repo from another remote URL.
        // Cargo actually does something similar for the out-dirs though it uses the
        // deprecated `std::hash::SipHasher` instead.
        let mut hasher = DefaultHasher::new();
        self.repo_url.as_ref()?.hash(&mut hasher);
        Some(format!("{:x}", hasher.finish()))
    }

    /// Translate the ref name to a directory name.
    ///
    /// This heaviliy sanitizes that name as it translates an arbitrary git tag, branch or
    /// commit to a folder name, as such we allow only alphanumeric ASCII characters and
    /// most punctuation.
    fn repo_dir(&self) -> String {
        // Most of the time this returns either a tag in the form of `v<version>` or a
        // branch name like `release/v<version>`, implementing special logic to prevent
        // the very rare case that a tag and branch with the same name exists is not worth
        // it and can also be worked around without this logic.
        let ref_name = match &self.git_ref {
            git::Ref::Branch(n) | git::Ref::Tag(n) | git::Ref::Commit(n) => n,
        };
        // Replace all directory separators with a dash `-`, so that we don't create
        // subfolders for tag or branch names that contain such characters.
        let mut ref_name = ref_name.replace(&['/', '\\'], "-");

        // Sanitize:
        // Remove all chars that are not ASCII alphanumeric or almost all
        // punctuation, except the ones forbidden in paths (more information here
        // https://stackoverflow.com/questions/1976007/what-characters-are-forbidden-in-windows-and-linux-directory-names).
        ref_name.retain(|c| {
            c.is_ascii_alphanumeric()
                || b"!#$%&'()+,-.;=@[]^_`{}~"
                    .iter()
                    .any(|delim| c == *delim as char)
        });
        ref_name
    }
}

/// Installer for the esp-idf source and tools.
pub struct Installer {
    esp_idf_origin: EspIdfOrigin,
    custom_install_dir: Option<PathBuf>,
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
            EspIdfOrigin::Managed(managed) => (managed.open_or_clone(&install_dir)?, true),
            EspIdfOrigin::Custom(repository) => (repository, false),
        };
        let version = EspIdfVersion::try_from(&repository);

        let path_var_sep = if cfg!(windows) { ';' } else { ':' };

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
        let tools = self
            .tools_provider
            .map(|p| p(&repository, &version))
            .unwrap_or(Ok(Vec::new()))?;
        let mut exported_paths = HashSet::new();
        for tool in tools {
            let tools_json = tool
                .index
                .as_ref()
                .map(|tools_json| [OsStr::new("--tools-json"), tools_json.as_os_str()].into_iter())
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
            venv_python: python,
            version,
            is_managed_espidf: managed_repo,
        })
    }

    /// Get the global install dir
    ///
    /// Panics if the OS does not provide a home directory.
    fn global_install_dir() -> PathBuf {
        dirs::home_dir()
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
            Some('v') if version.len() > 1 && version.chars().nth(1).unwrap().is_ascii_digit() => {
                git::Ref::Tag(version.to_owned())
            }
            Some(_) => git::Ref::Branch(version.to_owned()),
            _ => unreachable!(),
        },
    }
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
