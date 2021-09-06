use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Error};
use serde::Deserialize;

use super::codemodel::Language;
use super::{index, ObjKind, Version};

#[derive(Debug, Clone, Deserialize)]
pub struct Toolchains {
    pub version: Version,
    pub toolchains: Vec<Toolchain>,
}

impl TryFrom<&index::Reply> for Toolchains {
    type Error = Error;
    fn try_from(value: &index::Reply) -> Result<Self, Self::Error> {
        assert!(value.kind == ObjKind::Toolchains);
        assert!(value.version.major == ObjKind::Toolchains.expected_major_version());

        serde_json::from_reader(&fs::File::open(&value.json_file)?).with_context(|| {
            anyhow!(
                "Parsing cmake-file-api toolchains object file '{}' failed",
                value.json_file.display()
            )
        })
    }
}

impl Toolchains {
    pub fn get(&self, lang: Language) -> Option<&Toolchain> {
        self.toolchains.iter().find(|t| t.language == lang)
    }

    pub fn take(&mut self, lang: Language) -> Option<Toolchain> {
        let (i, _) = self
            .toolchains
            .iter()
            .enumerate()
            .find(|(_, t)| t.language == lang)?;
        Some(self.toolchains.swap_remove(i))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Toolchain {
    pub language: Language,
    pub compiler: Compiler,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compiler {
    pub path: Option<PathBuf>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub source_file_extensions: Vec<String>,
}
