use std::collections::HashMap;
use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codemodel::Codemodel;
use super::{Query, Version};

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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    pub multi_config: bool,
    pub name: String,
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CMake {
    pub version: Version,
    pub paths: HashMap<PathsKey, String>,
    pub generator: Generator,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize, Clone, Copy, Hash)]
#[serde(rename_all = "camelCase")]
pub enum ObjKind {
    Codemodel,
    Cache,
    CmakeFiles,
    Toolchains,
}

impl ObjKind {
    pub fn expected_major_version(self) -> u32 {
        match self {
            Self::Codemodel => 2,
            Self::Cache => 2,
            Self::CmakeFiles => 1,
            Self::Toolchains => 1,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub json_file: PathBuf,
    pub kind: ObjKind,
    pub version: Version,
}

impl Reply {
    pub fn codemodel(&self) -> Result<Codemodel> {
        Codemodel::try_from(self)
    }
}

#[derive(Debug, Clone)]
pub struct Replies {
    pub cmake: CMake,
    pub replies: HashMap<ObjKind, Reply>,
}

impl Replies {
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
                    "No cmake-file-api index file found in '{}'",
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

        let client = format!("client-{}", &query.client_name);
        let (_, reply) = reply
            .into_iter()
            .find(|(k, _)| k == &client)
            .ok_or_else(|| anyhow!("Reply for client '{}' not found.", &query.client_name))
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
                .filter_map(|(k, v)| match v {
                    ReplyOrError::Reply(mut r) => {
                        let expected_major_version = r.kind.expected_major_version();
                        if expected_major_version == r.version.major {
                            r.json_file = reply_dir.join(r.json_file);
                            Some((r.kind, r))
                        } else {
                            errors.push(format!(
                                "Object version missmatch for '{}': expected v{} got v{}",
                                k, expected_major_version, r.version.major
                            ));
                            None
                        }
                    }
                    ReplyOrError::Error { error } => {
                        errors.push(error);
                        None
                    }
                })
                .collect();

        if replies.is_empty() {
            let error = base_error().context("No valid reply objects found.");
            if !errors.is_empty() {
                return Err(error.context(errors.join(",\n")));
            } else {
                return Err(error);
            }
        }

        Ok(Replies { cmake, replies })
    }

    pub fn get_kind(&self, kind: ObjKind) -> Result<&Reply> {
        self.replies
            .get(&kind)
            .ok_or_else(|| anyhow!("Object {:?} not fund in cmake-file-api reply index.", kind))
    }

    pub fn get_codemodel(&self) -> Result<Codemodel> {
        self.get_kind(ObjKind::Codemodel)?.codemodel()
    }
}
