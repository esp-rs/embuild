//! Cache cmake file API object.

use std::convert::TryFrom;
use std::fs;

use anyhow::{anyhow, Context, Error};
use serde::Deserialize;

use super::{index, ObjKind, Version};

/// The variables stored in the persistent cache (`CMakeCache.txt`) for the build tree.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Cache {
    /// The version of this object kind.
    pub version: Version,
    /// All cache entries.
    pub entries: Vec<Entry>,
}

impl TryFrom<&index::Reply> for Cache {
    type Error = Error;
    fn try_from(value: &index::Reply) -> Result<Self, Self::Error> {
        assert!(value.kind == ObjKind::Cache);
        ObjKind::Cache
            .check_version_supported(value.version.major)
            .unwrap();

        serde_json::from_reader(&fs::File::open(&value.json_file)?).with_context(|| {
            anyhow!(
                "Parsing cmake-file-api cache object file '{}' failed",
                value.json_file.display()
            )
        })
    }
}

/// A cmake cache (`CMakeCache.txt`) entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Entry {
    /// The name of the entry.
    pub name: String,
    /// The value of the entry.
    pub value: String,
    /// The type of the entry.
    #[serde(rename = "type")]
    pub entry_type: Type,
    /// Properties set for this entries.
    pub properties: Vec<Property>,
}

/// The type of entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "String")]
pub enum Type {
    Bool,
    Path,
    Filepath,
    String,
    Internal,
    Static,
    Uninitialized,
    Other(String),
}

impl From<String> for Type {
    fn from(s: String) -> Self {
        match s.as_str() {
            "BOOL" => Self::Bool,
            "PATH" => Self::Path,
            "FILEPATH" => Self::Filepath,
            "STRING" => Self::String,
            "INTERNAL" => Self::Internal,
            "STATIC" => Self::Static,
            "UNINITIALIZED" => Self::Uninitialized,
            _ => Self::Other(s),
        }
    }
}

/// A property set for an [`Entry`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE", tag = "name", content = "value")]
pub enum Property {
    Advanced(String),
    Helpstring(String),
    Modified(String),
    Strings(String),
    Type(Type),
    Value(String),
    #[serde(other)]
    Unknown,
}
