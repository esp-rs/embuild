//! CMake file API and other utilities.

use std::collections::HashMap;
use std::convert::TryFrom;
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::{Error, Result};
use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::build::{CInclArgs, LinkArgsBuilder};
use crate::cli::NativeCommandArgs;
use crate::cmd;

pub mod file_api;
pub use dep_cmake::*;
pub use file_api::Query;

/// An enum for parsing and passing to cmake the standard command-line generators.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, EnumString, Display, EnumIter, IntoStaticStr)]
#[strum(ascii_case_insensitive)]
pub enum Generator {
    Ninja,
    NinjaMultiConfig,
    UnixMakefiles,
    BorlandMakefiles,
    MSYSMakefiles,
    MinGWMakefiles,
    NMakeMakefiles,
    NMakeMakefilesJOM,
    WatcomWMake,
}

impl Generator {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ninja => "Ninja",
            Self::NinjaMultiConfig => "Ninja MultiConfig",
            Self::UnixMakefiles => "Unix Makefiles",
            Self::BorlandMakefiles => "Borland Makefiles",
            Self::MSYSMakefiles => "MSYS Makefiles",
            Self::MinGWMakefiles => "MinGW Makefiles",
            Self::NMakeMakefiles => "NMake Makefiles",
            Self::NMakeMakefilesJOM => "NMake Makefiles JOM",
            Self::WatcomWMake => "Watcom WMake",
        }
    }
}

/// Get all variables defined in the `cmake_script_file`.
///
/// #### Note
/// This will run the script using `cmake -P`, beware of any side effects. Variables that
/// cmake itself sets will also be returned.
pub fn get_script_variables(
    cmake_script_file: impl AsRef<Path>,
) -> Result<HashMap<String, String>> {
    let temp_file = script_variables_extractor(cmake_script_file)?;

    let output = cmd!(cmake(), "-P", temp_file.as_ref()).stdout()?;
    drop(temp_file);

    let output = process_script_variables_extractor_output(output)?;

    Ok(output)
}

/// Augment a `cmake_script_file` with code that will extract all variables from the script
/// when executing the augmented script. Variables that cmake itself sets will also be returned.
///
/// #### Note
/// Run the returned script using `cmake -P`, beware of any side effects.
pub fn script_variables_extractor(cmake_script_file: impl AsRef<Path>) -> Result<impl AsRef<Path>> {
    let mut temp_file = tempfile::NamedTempFile::new()?;
    std::io::copy(&mut File::open(cmake_script_file)?, &mut temp_file)?;

    temp_file.write_all(
        r#"
message(STATUS "VARIABLE_DUMP_START")
get_cmake_property(_variableNames VARIABLES)
list (SORT _variableNames)
foreach (_variableName ${_variableNames})
    message(STATUS "${_variableName}=${${_variableName}}")
endforeach()
    "#
        .as_bytes(),
    )?;

    temp_file.as_file().sync_all()?;

    Ok(temp_file.into_temp_path())
}

/// Process the provided stdout output from executing an augmented script as returned by
/// `script_variables_extractor` and return a map of all variables from that script.
pub fn process_script_variables_extractor_output(
    output: impl AsRef<str>,
) -> Result<HashMap<String, String>> {
    let output = output.as_ref();

    Ok(output
        .lines()
        .filter_map(|l| l.strip_prefix("-- "))
        .skip_while(|&l| l != "VARIABLE_DUMP_START")
        .skip(1)
        .map(|l| {
            if let Some((name, value)) = l.split_once('=') {
                (name.to_owned(), value.to_owned())
            } else {
                (l.to_owned(), String::new())
            }
        })
        .collect())
}

/// The cmake executable used.
pub fn cmake() -> OsString {
    env::var_os("CMAKE").unwrap_or_else(|| "cmake".into())
}

impl TryFrom<&file_api::codemodel::target::Link> for LinkArgsBuilder {
    type Error = Error;

    fn try_from(link: &file_api::codemodel::target::Link) -> Result<Self, Self::Error> {
        let linkflags = link
            .command_fragments
            .iter()
            .flat_map(|f| NativeCommandArgs::new(&f.fragment))
            .collect();
        Ok(LinkArgsBuilder {
            linkflags,
            ..Default::default()
        })
    }
}

impl TryFrom<&file_api::codemodel::target::CompileGroup> for CInclArgs {
    type Error = Error;

    fn try_from(value: &file_api::codemodel::target::CompileGroup) -> Result<Self, Self::Error> {
        let args = value
            .defines
            .iter()
            .map(|d| format!("-D{}", d.define))
            .chain(
                value
                    .includes
                    .iter()
                    .map(|i| format!("\"-isystem{}\"", i.path)),
            )
            .collect::<Vec<_>>()
            .join(" ");

        Ok(Self { args })
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn test_get_script_variables() {
        let mut script = tempfile::NamedTempFile::new().unwrap();
        write!(&mut script, "set(VAR \"some string\")").unwrap();

        let script_path = script.into_temp_path();
        let vars = get_script_variables(&script_path).unwrap();

        println!("{vars:?}");

        let var = vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .find(|&(k, _)| k == "VAR");
        assert_eq!(var, Some(("VAR", "some string")));
    }
}
