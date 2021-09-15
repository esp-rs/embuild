use std::collections::HashMap;
use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::cache::Cache;
use super::codemodel::Codemodel;
use super::toolchains::Toolchains;
use super::{Query, Version};

/// CMake tool kind for [`CMake::paths`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum PathsKey {
    #[serde(rename = "cmake")]
    CMake,
    #[serde(rename = "ctest")]
    CTest,
    #[serde(rename = "cpack")]
    CPack,
    #[serde(rename = "root")]
    Root,
}

/// The cmake generator used for the build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    /// Whether the generator supports multiple output configurations.
    pub multi_config: bool,
    /// The name of the generator.
    pub name: String,
    /// If the generator supports `CMAKE_GENERATOR_PLATFORM`, specifies the generator
    /// platform name.
    pub platform: Option<String>,
}

/// Information about the instance of CMake that generated the replies.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CMake {
    pub version: Version,
    pub paths: HashMap<PathsKey, String>,
    pub generator: Generator,
}

/// CMake file API object kind for which cmake should generate information about.
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Clone, Copy, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ObjKind {
    Codemodel,
    Cache,
    CmakeFiles,
    Toolchains,
}

impl ObjKind {
    /// Get the supported major version of this object kind.
    pub(crate) const fn supported_version(self) -> u32 {
        match self {
            Self::Codemodel => 2,
            Self::Cache => 2,
            Self::CmakeFiles => 1,
            Self::Toolchains => 1,
        }
    }

    /// Check if `object_version` is supported by this library.
    pub fn check_version_supported(self, object_version: u32) -> Result<()> {
        let expected_version = self.supported_version();
        if object_version != expected_version {
            bail!(
                "cmake {} object version not supported (expected {}, got {})",
                self.as_str(),
                expected_version,
                object_version
            );
        } else {
            Ok(())
        }
    }

    /// Get the minimum required cmake version for this object kind.
    pub fn min_cmake_version(self) -> Version {
        let (major, minor) = match self {
            Self::Codemodel => (3, 14),
            Self::Cache => (3, 14),
            Self::CmakeFiles => (3, 14),
            Self::Toolchains => (3, 20),
        };
        Version {
            major,
            minor,
            ..Version::default()
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codemodel => "codemodel",
            Self::Cache => "cache",
            Self::CmakeFiles => "cmakeFiles",
            Self::Toolchains => "toolchains",
        }
    }
}

/// A reply for a specific object kind.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    /// Path to the JSON file which contains the object.
    pub json_file: PathBuf,
    /// The kind of cmake file API object.
    pub kind: ObjKind,
    /// The version of the generated object.
    pub version: Version,
}

impl Reply {
    /// Try to load this reply as a codemodel object.
    pub fn codemodel(&self) -> Result<Codemodel> {
        Codemodel::try_from(self)
    }

    /// Try to load this reply as a cache object.
    pub fn cache(&self) -> Result<Cache> {
        Cache::try_from(self)
    }

    /// Try to load this reply as a toolchains object.
    pub fn toolchains(&self) -> Result<Toolchains> {
        Toolchains::try_from(self)
    }
}

/// Replies generated from a cmake file API query.
#[derive(Debug, Clone)]
pub struct Replies {
    /// Information about the instance of CMake that generated the replies.
    pub cmake: CMake,
    /// All generated replies.
    pub replies: HashMap<ObjKind, Reply>,
}

impl Replies {
    /// Try to load the cmake file API index from the query and validate.
    pub fn from_query(query: &Query) -> Result<Replies> {
        let reply_dir = query.api_dir.join("reply");

        let index_file = fs::read_dir(&reply_dir)
            .context("Failed to list cmake-file-api reply directory")?
            .filter_map(
                |file| match (&file, file.as_ref().ok().and_then(|f| f.file_type().ok())) {
                    (Ok(f), Some(file_type))
                        if file_type.is_file()
                            && f.file_name().to_string_lossy().starts_with("index-") =>
                    {
                        Some(f.path())
                    }
                    _ => None,
                },
            )
            .max()
            .ok_or_else(|| {
                anyhow!(
                    "No cmake-file-api index file found in '{}' \
                     (cmake version must be at least 3.14)",
                    reply_dir.display()
                )
            })?;

        #[derive(Deserialize)]
        struct Index {
            cmake: CMake,
            reply: HashMap<String, Value>,
        }

        let base_error = || {
            anyhow!(
                "Failed to parse the cmake-file-api index file '{}'",
                index_file.display()
            )
        };
        let Index { cmake, reply } =
            serde_json::from_reader(&fs::File::open(&index_file)?).with_context(&base_error)?;

        for kind in query.kinds {
            let min_cmake_version = kind.min_cmake_version();
            if cmake.version.major < min_cmake_version.major
                || cmake.version.minor < min_cmake_version.minor
            {
                bail!(
                    "cmake-file-api {} object not supported: cmake version missmatch, \
                      expected at least version {}, got version {} instead",
                    kind.as_str(),
                    min_cmake_version,
                    &cmake.version
                );
            }
        }

        let client = format!("client-{}", &query.client_name);
        let (_, reply) = reply
            .into_iter()
            .find(|(k, _)| k == &client)
            .ok_or_else(|| anyhow!("Reply for client '{}' not found", &query.client_name))
            .with_context(&base_error)?;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ReplyOrError {
            Reply(Reply),
            Error { error: String },
        }

        let mut errors = vec![];
        let replies: HashMap<ObjKind, Reply> =
            serde_json::from_value::<HashMap<String, ReplyOrError>>(reply)
                .with_context(&base_error)?
                .into_iter()
                .filter_map(|(_, v)| match v {
                    ReplyOrError::Reply(mut r) => {
                        if let Err(err) = r.kind.check_version_supported(r.version.major) {
                            errors.push(err.to_string());
                            None
                        } else {
                            r.json_file = reply_dir.join(r.json_file);
                            Some((r.kind, r))
                        }
                    }
                    ReplyOrError::Error { error } => {
                        errors.push(error);
                        None
                    }
                })
                .collect();

        let not_found = query
            .kinds
            .iter()
            .filter(|k| !replies.contains_key(k))
            .map(|k| k.as_str())
            .collect::<Vec<_>>();

        if !not_found.is_empty() {
            let error = anyhow!(
                "Objects {} could not be deserialized{}",
                not_found.join(", "),
                if errors.is_empty() {
                    String::new()
                } else {
                    format!(":\n{}", errors.join(",\n"))
                }
            );
            return Err(error
                .context(format!(
                    "Could not deserialize all requested objects ({:?})",
                    query.kinds
                ))
                .context(base_error()));
        } else if !errors.is_empty() {
            log::debug!(
                "Errors while deserializing cmake-file-api index `{:?}`: {}",
                index_file,
                errors.join(",\n")
            );
        }

        Ok(Replies { cmake, replies })
    }

    /// Get a reply of `kind`.
    pub fn get_kind(&self, kind: ObjKind) -> Result<&Reply> {
        self.replies.get(&kind).ok_or_else(|| {
            anyhow!(
                "Object {:?} (version {}) not fund in cmake-file-api reply index",
                kind,
                kind.supported_version()
            )
        })
    }

    /// Load the codemodel object from a codemodel reply.
    ///
    /// Convenience function for `get_kind(ObjKind::Codemodel)?.codemodel()`.
    pub fn get_codemodel(&self) -> Result<Codemodel> {
        self.get_kind(ObjKind::Codemodel)?.codemodel()
    }

    /// Load the cache object from a cache reply.
    ///
    /// Convenience function for `get_kind(ObjKind::Cache)?.cache()`.
    pub fn get_cache(&self) -> Result<Cache> {
        self.get_kind(ObjKind::Cache)?.cache()
    }

    /// Load the toolchains object from a toolchains reply.
    ///
    /// Convenience function for `get_kind(ObjKind::Toolchains)?.toolchains()`.
    pub fn get_toolchains(&self) -> Result<Toolchains> {
        self.get_kind(ObjKind::Toolchains)?.toolchains()
    }
}
