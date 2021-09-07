use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// Build a [`PathBuf`].
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use embuild::path_buf;
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
/// This is a simple wrapper over the [`std::process::Command`] API. It expects at least
/// one argument for the program to run. Every comma seperated argument thereafter is
/// added to the command's arguments. Arguments after an `@`-sign specify collections of
/// arguments (specifically `impl IntoIterator<Item = impl AsRef<OsStr>`). The opional
/// `key=value` arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// **Note:**
///  `@`-arguments must be followed by at least one normal argument. For example
/// `cmd!("cmd", @args)` will not compile but `cmd!("cmd", @args, "other")` will. You can
/// use `key=value` arguments to work around this limitation: `cmd!("cmd"; args=(args))`.
///
/// After building the command [`std::process::Command::spawn`] is called and its return
/// value returned.
///
/// # Examples
/// ```ignore
/// let args_list = ["--foo", "--bar", "value"];
/// cmd_spawn!("git", @args_list, "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd_spawn {
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*
        $($(builder. $k $v;)*)?

        builder.spawn()
    }};
}

/// Run a command to completion.
///
/// This is a simple wrapper over the [`std::process::Command`] API. It expects at least
/// one argument for the program to run. Every comma seperated argument thereafter is
/// added to the command's arguments. Arguments after an `@`-sign specify collections of
/// arguments (specifically `impl IntoIterator<Item = impl AsRef<OsStr>`). The opional
/// `key=value` arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// **Note:**
///  `@`-arguments must be followed by at least one normal argument. For example
/// `cmd!("cmd", @args)` will not compile but `cmd!("cmd", @args, "other")` will. You can
/// use `key=value` arguments to work around this limitation: `cmd!("cmd"; args=(args))`.
///
/// After building the command [`std::process::Command::status`] is called and its return
/// value returned if the command was executed sucessfully otherwise an error is returned.
/// If `status` is specified as the first `key=value` argument, the result of
/// [`Command::status`](std::process::Command::status) will be returned without checking
/// if the command succeeded.
///
/// # Examples
/// ```ignore
/// let args_list = ["--foo", "--bar", "value"];
/// cmd!("git", @args_list, "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd {
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)*; status, $($k:ident = $v:tt),*) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*
        $(builder. $k $v;)*

        use $crate::anyhow::Context;
        builder
            .status()
            .with_context(|| format!("Command '{:?}' failed to execute", &builder))
    }};
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*

        $($(builder. $k $v;)*)?

        match builder.status() {
            Err(err) => {
                Err(
                    $crate::anyhow::Error::new(err)
                        .context(format!("Command '{:?}' failed to execute", &builder))
                )
            },
            Ok(result) => {
                if !result.success() {
                    Err($crate::anyhow::anyhow!("Command '{:?}' failed with exit code {:?}.", &builder, result.code()))
                }
                else {
                    Ok(result)
                }
            }
        }
    }};
}

/// Run a command to completion and gets its `stdout` output.
///
/// This is a simple wrapper over the [`std::process::Command`] API. It expects at least
/// one argument for the program to run. Every comma seperated argument thereafter is
/// added to the command's arguments. Arguments after an `@`-sign specify collections of
/// arguments (specifically `impl IntoIterator<Item = impl AsRef<OsStr>`). The opional
/// `key=value` arguments after a semicolon are simply translated to calling the
/// `std::process::Command::<key>` method with `value` as its arguments.
///
/// **Note:**
///  `@`-arguments must be followed by at least one normal argument. For example
/// `cmd!("cmd", @args)` will not compile but `cmd!("cmd", @args, "other")` will. You can
/// use `key=value` arguments to work around this limitation: `cmd!("cmd"; args=(args))`.
///
/// After building the command [`std::process::Command::output`] is called. If the command
/// succeeded its `stdout` output is returned as a [`String`] otherwise an error is
/// returned. If `ignore_exitcode` is specified as the first `key=value` argument, the
/// command's output will be returned even if it ran unsuccessfully.
///
/// # Examples
/// ```ignore
/// let args_list = ["--foo", "--bar", "value"];
/// cmd_output!("git", @args_list, "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd_output {
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)*; ignore_exitcode $(,$k:ident = $v:tt)*) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*
        $(builder. $k $v;)*

        let result = builder.output()?;
        // TODO: add some way to quiet this output
        use std::io::Write;
        std::io::stdout().write_all(&result.stdout[..]).ok();
        std::io::stderr().write_all(&result.stderr[..]).ok();

        String::from_utf8_lossy(&result.stdout[..]).trim_end_matches(&['\n', '\r'][..]).to_string()
    }};
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*
        $($(builder. $k $v;)*)?

        match builder.output() {
            Err(err) => Err(err.into()),
            Ok(result) => {
                if !result.status.success() {
                    // TODO: add some way to quiet this output
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
            _ => bail!(
                "Failed to convert the OsString '{}' to string.",
                self.as_ref().to_string_lossy()
            ),
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
        bail!(
            "Server at url '{}' returned error status {}: {}",
            url,
            req.status(),
            req.status_text()
        );
    }

    let mut reader = req.into_reader();
    std::io::copy(&mut reader, writer)?;
    Ok(())
}
