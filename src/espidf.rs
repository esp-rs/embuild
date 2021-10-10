//! esp-idf source and tools installation.
//!
//! This modules enables discovering existing `esp-idf` installation and the corresponding
//! tools for an `esp-idf` version.
//! 
//! Currently, this does not try to be compatible with espressif's esp-idf and tools
//! installation, as that whould increase the complexity much more.
//! 
//! Right now, there are two locations where the `esp-idf` source and tools are
//! detected and installed:
//! - **`~/.embuild/espressif`**
//!
//!     This location is searched first for the esp-idf source when
//!     [`InstallOpts::FIND_PREFER_GLOBAL`] is set.
//!
//! - **`<crate root>/.embuild/espressif`**
//!
//! When [`InstallOpts::NO_GLOBAL_INSTALL`] is set the esp-idf source and tools are
//! installed inside the crate root if they could not be found in the global location and
//! are not installed already.
//!
//! ### Relavant env variables:
//! - `IDF_PATH`
//! - `CARGO_MANIFEST_DIR`
//! - `ESP_IDF_VERSION`
//! - `ESP_IDF_RESPOSITORY`

use std::path::{Path, PathBuf};

use bitflags::bitflags;
use anyhow::Result;

use crate::git;

const DEFAULT_ESP_IDF_REPOSITORY: &str = "https://github.com/espressif/esp-idf.git";
const DEFAULT_ESP_IDF_VERSION: &str = "v4.3";

/// The relative install dir of the `esp-idf` and its tools.
///
/// When installed globally it is relative to the user home directory,
/// otherwise it is relative to the crate root.
pub const INSTALL_DIR: &str = ".embuild/espressif";

/// One or more esp-idf tools.
#[derive(Debug, Clone)]
pub struct Tools {
    /// An optional path to the `tools.json` index to be used`.
    /// 
    /// This file is passed to the `tools.py` python script.
    pub index: Option<PathBuf>,
    /// All names of the tools that should be installed.
    pub tools: Vec<String>
}

/// Installer for the esp-idf source and tools.
#[derive(Debug, Clone)]
pub struct Installer {
    version: git::Ref,
    git_url: Option<String>,
    opts: InstallOpts,
    tools: Vec<Tools>
}

bitflags! {
    pub struct InstallOpts: u32 {
        const FIND_PREFER_GLOBAL = (1 << 0);
        const NO_GLOBAL_INSTALL = (1 << 1);
    }
}

pub struct EspIdfInfo {
    esp_idf_dir: PathBuf,
    esp_idf_version: git::Ref,
    exported_path: String,
    venv_python: PathBuf,
}

impl Installer {
    pub fn find_esp_idf(&self) -> Option<PathBuf> {
        let find = |base_dir: &Path| -> Option<PathBuf> {
            let install_dir = base_dir.join(INSTALL_DIR);
            if !install_dir.exists() {
                return None;
            }

            match self.version {
                git::Ref::Branch(ref s) | git::Ref::Tag(ref s) => {
                    let esp_idf_dir = install_dir.join(format!("esp-idf-{}", s));
                    if !esp_idf_dir.exists() {
                        None
                    } else {
                        Some(esp_idf_dir.to_owned())
                    }
                }
                git::Ref::Commit(ref c) => {
                    let full_dirname = format!("esp-idf-{}", c);
                    let mut esp_idf_dir = None;
                    // TODO: better error handling
                    for d in std::fs::read_dir(install_dir).ok()? {
                        if let Ok(d) = d {
                            let filename = d.file_name();
                            let dirname = match filename.to_str() {
                                Some(s) => s,
                                None => continue,
                            };

                            if dirname.starts_with(&full_dirname) {
                                esp_idf_dir = Some(d.path());
                                break;
                            }
                        }
                    }
                    esp_idf_dir
                }
            }
        };

        if self.opts.contains(InstallOpts::FIND_PREFER_GLOBAL) {
            dirs::home_dir()
                .and_then(|d| find(&d))
                .or_else(|| std::env::var_os("CARGO_MANIFEST_DIR").and_then(|d| find(Path::new(&d))))
        } else {
            std::env::var_os("CARGO_MANIFEST_DIR")
                .and_then(|d| find(Path::new(&d)))
                .or_else(|| dirs::home_dir().and_then(|d| find(&d)))
        }
    }
    
    pub fn install(self) -> Result<EspIdfInfo> {
        todo!()
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
