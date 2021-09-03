use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::cmd_output;

mod file_api;

pub use file_api::*;
pub use ::cmake::*;

/// Get all variables defined in the `cmake_script_file`.
///
/// #### Note
/// This will run the script using `cmake -P`, beware of any side effects. Variables that
/// cmake itself sets will also be returned.
pub fn get_script_variables(
    cmake_script_file: impl AsRef<Path>,
) -> Result<HashMap<String, String>> {
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
    let temp_file = temp_file.into_temp_path();

    let output = cmd_output!(cmake(), "-P", &temp_file)?;
    drop(temp_file);

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
    env::var_os("CMAKE").unwrap_or("cmake".into())
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

        println!("{:?}", vars);

        let var = vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .find(|&(k, _)| k == "VAR");
        assert_eq!(var, Some(("VAR", "some string")));
    }
}
