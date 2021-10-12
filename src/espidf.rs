//! esp-idf source and tools installation.
//!
//! This modules enables discovering existing `esp-idf` installation and the corresponding
//! tools for an `esp-idf` version.
//!
//! Right now, there are two locations where the `esp-idf` source and tools are
//! detected and installed:
//! - **`~/.espressif`**
//!
//!     This location is searched first for the esp-idf source when
//!     [`InstallOpts::FIND_PREFER_GLOBAL`] is set.
//!
//! - **`<crate root>/.espressif`**
//!
//! When [`InstallOpts::NO_GLOBAL_INSTALL`] is set the esp-idf source and tools are
//! installed inside the crate root even if they are already installed in the global
//! location.
//!
//! ### Relavant env variables:
//! - `IDF_PATH`
//! - `OUT_DIR`
//! - `ESP_IDF_VERSION`
//! - `ESP_IDF_RESPOSITORY`

use std::borrow::Cow;
use std::collections::HashSet;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use bitflags::bitflags;

use crate::python::PYTHON;
use crate::utils::PathExt;
use crate::{cmd, cmd_output, git, path_buf, python};

const DEFAULT_ESP_IDF_REPOSITORY: &str = "https://github.com/espressif/esp-idf.git";

/// The relative install dir of the `esp-idf` and its tools.
///
/// When installed globally it is relative to the user home directory,
/// otherwise it is relative to the crate root.
pub const INSTALL_DIR: &str = ".espressif";

/// One or more esp-idf tools.
#[derive(Debug, Clone)]
pub struct Tools {
    /// An optional path to the `tools.json` index to be used`.
    ///
    /// This file is passed to the `tools.py` python script.
    pub index: Option<PathBuf>,
    /// All names of the tools that should be installed.
    pub tools: Vec<String>,
}

impl Tools {
    pub fn new(iter: impl IntoIterator<Item = impl AsRef<str>>) -> Tools {
        Tools {
            index: None,
            tools: iter.into_iter().map(|s| s.as_ref().to_owned()).collect(),
        }
    }

    pub fn new_custom(
        iter: impl IntoIterator<Item = impl AsRef<str>>,
        tools_json: impl AsRef<Path>,
    ) -> Tools {
        Tools {
            index: Some(tools_json.as_ref().into()),
            tools: iter.into_iter().map(|s| s.as_ref().to_owned()).collect(),
        }
    }

    /// Create a tools instance for installing cmake 3.20.3.
    ///
    /// For this to work you must make sure that the second value only gets dropped
    /// after the returned [`Tools`] value has been used.
    pub fn cmake() -> Result<(Tools, impl Drop)> {
        let mut temp = tempfile::NamedTempFile::new()?;
        temp.as_file_mut()
            .write_all(include_bytes!("espidf/resources/cmake.json"))?;
        let temp = temp.into_temp_path();

        Ok((
            Tools {
                index: Some(temp.to_path_buf()),
                tools: vec!["cmake".into()],
            },
            temp,
        ))
    }
}

/// Installer for the esp-idf source and tools.
#[derive(Debug, Clone)]
pub struct Installer {
    version: git::Ref,
    git_url: Option<String>,
    opts: InstallOpts,
    tools: Vec<Tools>,
}

bitflags! {
    pub struct InstallOpts: u32 {
        const FIND_PREFER_GLOBAL = (1 << 0);
        const NO_GLOBAL_INSTALL = (1 << 1);
    }
}

pub struct EspIdfInfo {
    /// The esp-idf repository with version `esp_idf_version`.
    pub esp_idf: git::Repository,
    /// The [`git::Ref`] checked out in the esp-idf repository.
    pub esp_idf_version: git::Ref,
    /// The binary paths of all tools concatenated with the system `PATH` env variable.
    pub exported_path: OsString,
    /// The path to the python executable in the python virtual env.
    pub venv_python: PathBuf,
}

impl Installer {
    /// Create a new installer for the `esp_idf_version`.
    pub fn new(esp_idf_version: git::Ref) -> Installer {
        Installer {
            version: esp_idf_version,
            git_url: None,
            opts: InstallOpts::all(),
            tools: vec![],
        }
    }

    /// Add `tools` to the list of tools to install.
    pub fn with_tools(mut self, tools: Tools) -> Self {
        self.tools.push(tools);
        self
    }

    /// Set the install options to `opts`.
    pub fn opts(mut self, opts: InstallOpts) -> Self {
        self.opts = opts;
        self
    }

    /// Use `esp_idf_git_url` when cloning the `esp-idf`.
    pub fn git_url(mut self, esp_idf_git_url: String) -> Self {
        self.git_url = Some(esp_idf_git_url);
        self
    }

    fn esp_idf_folder_name(&self) -> Cow<'static, str> {
        const BASE_NAME: &str = "esp-idf";
        match self.version {
            git::Ref::Branch(ref s) | git::Ref::Tag(ref s) => format!("{}-{}", BASE_NAME, s).into(),
            git::Ref::Commit(_) => BASE_NAME.into(),
        }
    }

    /// Find a possible installed esp-idf git repository.
    ///
    /// This will search in two locations:
    /// - `<cargo crate root>/.espressif`
    /// - `~/.espressif`
    ///
    /// If [`InstallOpts::FIND_PREFER_GLOBAL`] is set, the global install location
    /// (`~/.espressif`) is looked into first.
    pub fn find_esp_idf(&self) -> Option<git::Repository> {
        let find = |base_dir: &Path| -> Option<git::Repository> {
            let install_dir = base_dir.join(INSTALL_DIR);
            if !install_dir.exists() {
                return None;
            }

            let esp_idf_dir = install_dir.join(self.esp_idf_folder_name().as_ref());
            if let Ok(repo) = git::Repository::open(&esp_idf_dir) {
                if repo.is_ref(&self.version) {
                    return Some(repo);
                }
            }
            None
        };

        if self.opts.contains(InstallOpts::FIND_PREFER_GLOBAL) {
            dirs::home_dir()
                .and_then(|d| find(&d))
                .or_else(|| crate_workspace_dir().and_then(|d| find(&d)))
        } else {
            crate_workspace_dir()
                .and_then(|d| find(&d))
                .or_else(|| dirs::home_dir().and_then(|d| find(&d)))
        }
    }

    /// Install the esp-idf source and all tools added with [`with_tools`](Self::with_tools).
    ///
    /// If [`InstallOpts::NO_GLOBAL_INSTALL`] is set this will install the esp-idf into
    /// the folder `<cargo crate root>/.espressif`, note that this will only work if this
    /// function is called inside a cargo build script (where the env variable `OUT_DIR`
    /// is set).
    /// Otherwise the global install directory `~/.espressif` is used.
    ///
    /// Installation will do the following things in order:
    /// - Try to find an installed esp-idf matching the specified version using
    ///   [`find_esp_idf`](Self::find_esp_idf).
    /// - If not found, clone it into `<install directory>/esp-idf<-version suffix>` where
    ///   `version suffix` is the branch name, tag name, or no suffix when a specific
    ///   commit hash is used.
    /// - Create a python virtual env using the system `python` and `idf_tools.py
    ///   install-python-env` in the install directory.
    /// - Install all tools with `idf_tools.py --tools-json <tools_json> install <tools...>`
    ///   per [`Tools`] instance added with [`with_tools`](Self::with_tools). `tools_json`
    ///   is the optional [`Tools::index`] path, if [`None`] the `tools.json` of the
    ///   esp-idf is used.
    pub fn install(self) -> Result<EspIdfInfo> {
        let sdk_dir = if self.opts.contains(InstallOpts::NO_GLOBAL_INSTALL) {
            PathBuf::from(crate_workspace_dir().ok_or_else(|| {
                anyhow!("Forced local install while outside of cargo build script")
            })?)
        } else {
            PathBuf::from(dirs::home_dir().ok_or_else(|| anyhow!("No system home directory"))?)
        }
        .join(INSTALL_DIR);

        let esp_idf = self.find_esp_idf().map_or_else(
            || {
                let esp_idf_dir = sdk_dir.join(self.esp_idf_folder_name().as_ref());

                self.clone_esp_idf(&esp_idf_dir)
            },
            Ok,
        )?;

        // This is a workaround for msys or even git bash.
        // When using them `idf_tools.py` prints unix paths (ex. `/c/user/` instead of
        // `C:\user\`), so we correct this with an invocation of `cygpath` which converts the
        // path to the windows representation.
        let cygpath_works = cfg!(windows) && cmd_output!("cygpath", "--version").is_ok();
        let to_win_path = if cygpath_works {
            |p: String| cmd_output!("cygpath", "-w", p).unwrap()
        } else {
            |p: String| p
        };
        let path_var_sep = if cygpath_works || cfg!(not(windows)) {
            ':'
        } else {
            ';'
        };

        // Create python virtualenv or use a previously installed one.

        // TODO: also install python
        python::check_python_at_least(3, 7)?;

        let idf_tools_py = path_buf![esp_idf.worktree(), "tools", "idf_tools.py"];

        let get_python_env_dir = || -> Result<String> {
            Ok(cmd_output!(PYTHON, &idf_tools_py, "--idf-path", esp_idf.worktree(), "--quiet", "export", "--format=key-value";
                       ignore_exitcode, env=("IDF_TOOLS_PATH", &sdk_dir))?
                            .lines()
                            .find(|s| s.trim_start().starts_with("IDF_PYTHON_ENV_PATH="))
                            .ok_or_else(|| anyhow!("`idf_tools.py export` result contains no `IDF_PYTHON_ENV_PATH` item"))?
                            .trim()
                            .strip_prefix("IDF_PYTHON_ENV_PATH=").unwrap()
                                  .to_string())
        };

        let python_env_dir = get_python_env_dir().map(&to_win_path);
        let python_env_dir: PathBuf = if python_env_dir.is_err()
        || !Path::new(&python_env_dir.as_ref().unwrap()).exists() {
            cmd!(PYTHON, &idf_tools_py, "--idf-path", esp_idf.worktree(), "--quiet", "--non-interactive", "install-python-env";
                 env=("IDF_TOOLS_PATH", &sdk_dir))?;
            to_win_path(get_python_env_dir()?)
        } else {
            python_env_dir.unwrap()
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
            let tools_json = if let Some(tools_json) = &tool.index {
                Some(std::array::IntoIter::new([
                    OsStr::new("--tools-json"),
                    tools_json.as_os_str(),
                ]))
            } else {
                None
            }
            .into_iter()
            .flatten();

            // Install the tools.
            cmd!(python, &idf_tools_py, "--idf-path", esp_idf.worktree(), @tools_json.clone(), "install"; 
                 env=("IDF_TOOLS_PATH", &sdk_dir), args=(tool.tools))?;

            // Get the paths to the tools.
            exported_paths.extend(
                cmd_output!(python, &idf_tools_py, "--idf-path", esp_idf.worktree(), @tools_json, "--quiet", "export", "--format=key-value";
                                ignore_exitcode, env=("IDF_TOOLS_PATH", &sdk_dir))?
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
                .map(|s| PathBuf::from(to_win_path(s)))
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )?;

        Ok(EspIdfInfo {
            esp_idf,
            esp_idf_version: self.version,
            exported_path: paths,
            venv_python: python,
        })
    }

    /// Clone the `esp-idf` into `install_dir`.
    pub fn clone_esp_idf(&self, install_dir: &Path) -> Result<git::Repository> {
        let mut esp_idf_repo = git::Repository::new(install_dir);

        esp_idf_repo.clone_ext(
            self.git_url
                .as_deref()
                .unwrap_or(DEFAULT_ESP_IDF_REPOSITORY),
            git::CloneOptions::new()
                .force_ref(self.version.clone())
                .depth(1),
        )?;

        Ok(esp_idf_repo)
    }
}

/// Decode a [`git::Ref`] from an esp-idf version string.
///
/// The version string can have the following format:
/// - `commit:<hash>`: Uses the commit `<hash>` of the `esp-idf` repository. Note that
///                    this will clone the whole `esp-idf` not just one commit.
/// - `tag:<tag>`: Uses the tag `<tag>` of the `esp-idf` repository.
/// - `branch:<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
/// - `v<major>.<minor>` or `<major>.<minor>`: Uses the tag `v<major>.<minor>` of the `esp-idf` repository.
/// - `<branch>`: Uses the branch `<branch>` of the `esp-idf` repository.
pub fn decode_esp_idf_version_ref(version: &str) -> git::Ref {
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

/// Try to get the path to crate workspace dir or [`None`] if unavailable.
///
/// The crate workspace directory is the directory containing the cargo `target` dir in
/// which all artifacts for compilation are stored. We can only get the workspace
/// directory if we're currently running inside a cargo build script.
fn crate_workspace_dir() -> Option<PathBuf> {
    // We pop the path to the out dir 6 times to get to the workspace root so the
    // directory containing the `target` (build) directory. The directory containing the
    // `target` directory is always the workspace root.
    Some(PathBuf::from(env::var_os("OUT_DIR")?).pop_times(6))
}
