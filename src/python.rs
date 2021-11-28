use anyhow::{anyhow, Context, Result};

use crate::cmd_output;

#[cfg(windows)]
pub const PYTHON: &str = "python"; // No 'python3.exe' on Windows
#[cfg(not(windows))]
pub const PYTHON: &str = "python3";

/// Check that python is at least `major.minor`.
pub fn check_python_at_least(major: u32, minor: u32) -> Result<()> {
    let version_str = cmd_output!(PYTHON, "--version")
        .context("Failed to locate python. Is python installed and in your $PATH?")?;

    let base_err = || anyhow!("Unexpected output from {}", PYTHON);

    if !version_str.starts_with("Python ") {
        return Err(base_err().context("Expected a version string starting with 'Python '"));
    }

    let version_str = &version_str["Python ".len()..];
    let version = version_str
        .split('.')
        .map(|s| s.parse::<u32>().ok())
        .collect::<Vec<_>>();

    if version.len() < 2 || version[0].is_none() || version[1].is_none() {
        return Err(
            base_err().context("Expected a version string of type '<number>.<number>[.remainder]'")
        );
    }

    let python_major = version[0].unwrap();
    let python_minor = version[1].unwrap();

    if python_major < major || python_minor < minor {
        Err(anyhow!(
            "Invalid python version '{}'; expected at least {}.{}",
            version_str,
            major,
            minor
        )
        .context(format!("When running '{} --version'", PYTHON)))
    } else {
        Ok(())
    }
}
