use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::path_buf;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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

#[derive(Clone, Debug)]
pub struct Query {
    api_dir: PathBuf,
    client_name: String,
}

impl Query {
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
                kind.expected_major_version()
            )))?;
        }

        Ok(Query {
            api_dir,
            client_name,
        })
    }

    /// Try to get all replies from this query.
    pub fn get_replies(&self) -> Result<Replies> {
        Replies::from_query(self)
    }
}

pub mod cache;
pub mod codemodel;
pub mod toolchains;
mod index;

pub use cache::Cache;
pub use codemodel::Codemodel;
pub use index::*;
