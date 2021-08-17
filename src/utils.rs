use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Build a [`PathBuf`].
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// assert_eq!(path_buf!["/foo", "bar"].as_path(), Path::new("/foo/bar"));
/// ```
#[macro_export]
macro_rules! path_buf {
    ($($e: expr),*) => {{
        use std::path::PathBuf;
        let mut pb = PathBuf::new();
        $(
            pb.push($e);
        )*
        pb
    }}
}

/// Spawn a command and return its handle.
///
/// This is a simple wrapper over the [`std::process::Command`] API.
/// It expects at least one argument for the program to run. Every comma seperated
/// argument thereafter is added to the commands arguments. The opional `key=value`
/// arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// After building the command [`std::process::Command::spawn`] is called and its return
/// value returned.
///
/// # Examples
/// ```ignore
/// cmd_spawn("git", "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd_spawn {
    ($cmd:expr $(,$cmdarg:expr)*; $($k:ident = $v:tt),*) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(builder.arg($cmdarg);)*
        $(builder. $k $v;)*

        builder.spawn()
    }};
    ($cmd:expr $(,$cmdarg:expr)*) => {
        cmd_spawn!($cmd, $($cmdarg),*;)
    };
}

/// Run a command to completion.
///
/// This is a simple wrapper over the [`std::process::Command`] API.
/// It expects at least one argument for the program to run. Every comma seperated
/// argument thereafter is added to the commands arguments. The opional `key=value`
/// arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// After building the command [`std::process::Command::status`] is called and its return
/// value returned if the command was executed sucessfully otherwise an error is returned.
///
/// # Examples
/// ```ignore
/// cmd("git", "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd {
    ($cmd:expr $(,$cmdarg:expr)*; $($k:ident = $v:tt),*) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(builder.arg($cmdarg);)*
        $(builder. $k $v;)*

        match builder.status() {
            Err(err) => Err(err.into()),
            Ok(result) => {
                if !result.success() {
                    Err(anyhow::anyhow!("Command '{:?}' failed with exit code {:?}.", &builder, result.code()))
                }
                else {
                    Ok(result)
                }
            }
        }
    }};
    ($cmd:expr $(,$cmdarg:expr)*) => {
        cmd!($cmd, $($cmdarg),*;)
    };
}

/// Run a command to completion and gets its `stdout` output.
///
/// This is a simple wrapper over the [`std::process::Command`] API.
/// It expects at least one argument for the program to run. Every comma seperated
/// argument thereafter is added to the commands arguments. The opional `key=value`
/// arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// After building the command [`std::process::Command::output`] is called. If the command
/// succeeded its `stdout` output is returned as a [`String`] otherwise an error is returned.
/// If the first `ignore_exitcode` is specified as the first `key=value` argument, the
/// commands output will be returned even if it ran unsuccessfully.
///
/// # Examples
/// ```ignore
/// cmd("git", "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd_output {
    ($cmd:expr $(,$cmdarg:expr)*; ignore_exitcode $(,$k:ident = $v:tt)* ) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(builder.arg($cmdarg);)*
        $(builder. $k $v;)*

        let result = builder.output()?;
        use std::io::Write;
        std::io::stdout().write_all(&result.stdout[..]).ok();
        std::io::stderr().write_all(&result.stderr[..]).ok();
        String::from_utf8_lossy(&result.stdout[..]).trim_end_matches(&['\n', '\r'][..]).to_string()
    }};
    ($cmd:expr $(,$cmdarg:expr)*; $($k:ident = $v:tt),*) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(builder.arg($cmdarg);)*
        $(builder. $k $v;)*

        match builder.output() {
            Err(err) => Err(err.into()),
            Ok(result) => {
                if !result.status.success() {
                    use std::io::Write;
                    std::io::stdout().write_all(&result.stdout[..]).ok();
                    std::io::stderr().write_all(&result.stderr[..]).ok();

                    Err(anyhow::anyhow!("Command '{:?}' failed with exit code {:?}.", &builder, result.status.code()))
                }
                else {
                    Ok(String::from_utf8_lossy(&result.stdout[..]).trim_end_matches(&['\n', '\r'][..]).to_string())
                }
            }
        }
    }};
    ($cmd:expr $(,$cmdarg:expr)*) => {
        cmd_output!($cmd, $($cmdarg),*;)
    };
}

pub trait PathExt: AsRef<Path> {
    /// Pop `times` segments from this path.
    fn pop_times(&self, times: usize) -> PathBuf {
        let mut path: PathBuf = self.as_ref().to_owned();
        for _ in 0..times {
            path.pop();
        }
        path
    }

    /// Make this path absolute relative to `relative_dir` if not already.
    /// 
    /// Note: Does not check if the path exists and no normalization takes place.
    fn abspath_relative_to(&self, relative_dir: impl AsRef<Path>) -> PathBuf {
        if self.as_ref().is_absolute() {
            return self.as_ref().to_owned();
        }

        relative_dir.as_ref().join(self)
    }

    /// Make this path absolute relative to [`env::current_dir`] if not already.
    /// 
    /// Note: Does not check if the path exists and no normalization takes place.
    fn abspath(&self) -> Result<PathBuf> {
        if self.as_ref().is_absolute() {
            return Ok(self.as_ref().to_owned());
        }

        Ok(env::current_dir()?.join(self))
    }
}

impl PathExt for Path {}
impl PathExt for PathBuf {}

pub trait OsStrExt: AsRef<OsStr> {
    /// Try to convert this [`OsStr`] into a string.
    fn try_to_str(&self) -> Result<&str> {
        match self.as_ref().to_str() {
            Some(s) => Ok(s),
            _ => bail!("Failed to convert the OsString '{}' to string.", self.as_ref().to_string_lossy())
        }
    }
}

impl OsStrExt for OsStr {}
impl OsStrExt for std::ffi::OsString {}
impl OsStrExt for Path {}
impl OsStrExt for PathBuf {}

/// Download the contents of `url` to `writer`.
pub fn download_file_to(url: &str, writer: &mut impl std::io::Write) -> Result<()> {
    let req = ureq::get(url).call()?;
    if req.status() != 200 {
        bail!("Server at url '{}' returned error status {}: {}",url, req.status(), req.status_text());
    }
    
    let mut reader = req.into_reader();
    std::io::copy(&mut reader, writer)?;
    Ok(())
}