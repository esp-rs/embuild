//! Toolchains cmake file API object.

use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Error};
use serde::Deserialize;

use super::codemodel::Language;
use super::{index, ObjKind, Version};

/// Toolchain object.
///
/// The toolchains object kind lists properties of the toolchains used during the build.
/// These include the language, compiler path, ID, and version.
#[derive(Debug, Clone, Deserialize)]
pub struct Toolchains {
    /// Version of the object kind.
    pub version: Version,
    /// A list of toolchains associated with a particular language.
    pub toolchains: Vec<Toolchain>,
}

impl TryFrom<&index::Reply> for Toolchains {
    type Error = Error;
    fn try_from(value: &index::Reply) -> Result<Self, Self::Error> {
        assert!(value.kind == ObjKind::Toolchains);
        ObjKind::Toolchains
            .check_version_supported(value.version.major)
            .unwrap();

        serde_json::from_reader(&fs::File::open(&value.json_file)?).with_context(|| {
            anyhow!(
                "Parsing cmake-file-api toolchains object file '{}' failed",
                value.json_file.display()
            )
        })
    }
}

impl Toolchains {
    /// Get the toolchain assosicated with language `lang`.
    pub fn get(&self, lang: Language) -> Option<&Toolchain> {
        self.toolchains.iter().find(|t| t.language == lang)
    }

    /// Take the toolchain assosicated with language `lang`.
    pub fn take(&mut self, lang: Language) -> Option<Toolchain> {
        let (i, _) = self
            .toolchains
            .iter()
            .enumerate()
            .find(|(_, t)| t.language == lang)?;
        Some(self.toolchains.swap_remove(i))
    }
}

/// A toolchain associated with a particular language.
#[derive(Debug, Clone, Deserialize)]
pub struct Toolchain {
    /// The associated programming language of this toolchain.
    pub language: Language,
    /// The compiler of this toolchain.
    pub compiler: Compiler,
}

/// The compiler of a toolchain and language.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compiler {
    /// The path to the compiler's executable.
    pub path: Option<PathBuf>,
    /// The ID of the compiler.
    pub id: Option<String>,
    /// The version of the compiler.
    pub version: Option<String>,
    /// The cross-compiling target of the compiler.
    pub target: Option<String>,
    /// A list of file extensions (without the leading dot) for the language's
    /// source files (empty if not preset).
    #[serde(default)]
    pub source_file_extensions: Vec<String>,
}
