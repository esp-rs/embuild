//! The [cmake file
//! API](https://cmake.org/cmake/help/git-stage/manual/cmake-file-api.7.html) used to get
//! information about the build-system and build.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::path_buf;

/// An object or cmake version.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    #[serde(default)]
    pub patch: u32,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub is_dirty: bool,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}{}{}",
            self.major,
            self.minor,
            self.patch,
            if !self.suffix.is_empty() { "-" } else { "" },
            self.suffix
        )
    }
}

/// The query for the cmake-file-api.
#[derive(Clone, Debug)]
pub struct Query<'a> {
    api_dir: PathBuf,
    client_name: String,
    kinds: &'a [ObjKind],
}

impl Query<'_> {
    /// Create a new query.
    pub fn new(
        cmake_build_dir: impl AsRef<Path>,
        client_name: impl Into<String>,
        kinds: &[ObjKind],
    ) -> Result<Query> {
        let client_name = client_name.into();
        let api_dir = path_buf![cmake_build_dir, ".cmake", "api", "v1"];

        let client_dir = path_buf![&api_dir, "query", format!("client-{}", &client_name)];
        fs::create_dir_all(&client_dir)?;

        for kind in kinds {
            fs::File::create(client_dir.join(format!(
                "{}-v{}",
                kind.as_str(),
                kind.supported_version()
            )))?;
        }

        Ok(Query {
            api_dir,
            client_name,
            kinds,
        })
    }

    /// Try to get all replies from this query.
    pub fn get_replies(&self) -> Result<Replies> {
        Replies::from_query(self)
    }
}

pub mod cache;
pub mod codemodel;
mod index;
pub mod toolchains;

pub use cache::Cache;
pub use codemodel::Codemodel;
pub use index::*;
pub use toolchains::Toolchains;
