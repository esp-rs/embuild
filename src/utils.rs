use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::{env, io, process};

use anyhow::Result;

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

/// Error when trying to execute a command.
#[derive(Debug, thiserror::Error)]
pub enum CmdError {
    /// The command failed to start.
    #[error("command '{0}' failed to start")]
    NoRun(String, #[source] io::Error),
    /// The command exited unsucessfully (with non-zero exit status).
    #[error("command '{0}' exited with non-zero status code {1}")]
    Unsuccessful(String, i32, #[source] Option<anyhow::Error>),
    /// The command was terminated unexpectedly.
    #[error("command '{0}' was terminated unexpectedly")]
    Terminated(String),
}

impl CmdError {
    /// Create a [`CmdError::NoRun`].
    pub fn no_run(cmd: &process::Command, error: io::Error) -> Self {
        CmdError::NoRun(format!("{:?}", cmd), error)
    }

    /// Convert a [`process::ExitStatus`] into a `Result<(), CmdError>`.
    pub fn status_into_result(
        status: process::ExitStatus,
        cmd: &process::Command,
        cmd_output: impl FnOnce() -> Option<String>,
    ) -> Result<(), Self> {
        if status.success() {
            Ok(())
        } else if let Some(code) = status.code() {
            Err(CmdError::Unsuccessful(
                format!("{:?}", cmd),
                code,
                cmd_output().map(anyhow::Error::msg),
            ))
        } else {
            Err(CmdError::Terminated(format!("{:?}", cmd)))
        }
    }
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

        builder
            .status()
            .map_err(|e| $crate::utils::CmdError::no_run(&builder, e))
    }};
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*

        $($(builder. $k $v;)*)?

        use $crate::utils::CmdError;

        builder
            .status()
            .map_err(|e| CmdError::no_run(&builder, e))
            .and_then(|v| CmdError::status_into_result(v, &builder, || None))
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
/// command's output will be returned without checking if the command succeeded.
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

        builder.output()
            .map_err(|e| $crate::utils::CmdError::no_run(&builder, e))
            .map(|result| {
                // TODO: add some way to quiet this output
                use std::io::Write;
                std::io::stdout().write_all(&result.stdout[..]).ok();
                std::io::stderr().write_all(&result.stderr[..]).ok();

                String::from_utf8_lossy(&result.stdout[..]).trim_end_matches(&['\n', '\r'][..]).to_string()
            })
    }};
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let cmd = &($cmd);
        let mut builder = std::process::Command::new(cmd);
        $(
            $(builder.args($cmdargs);)*
            builder.arg($cmdarg);
        )*
        $($(builder. $k $v;)*)?

        use $crate::utils::CmdError;
        match builder.output() {
            Err(err) => {
                Err(CmdError::no_run(&builder, err))
            },
            Ok(result) => {
                CmdError::status_into_result(result.status, &builder, || {
                    Some(
                         String::from_utf8_lossy(&result.stderr[..])
                            .trim_end()
                            .to_string()
                    )
                })
                .map_err(|e| {
                    // TODO: add some way to quiet this output
                    use std::io::Write;
                    std::io::stdout().write_all(&result.stdout[..]).ok();
                    std::io::stderr().write_all(&result.stderr[..]).ok();
                    e
                })
                .map(|_| {
                    String::from_utf8_lossy(&result.stdout[..]).trim_end_matches(&['\n', '\r'][..]).to_string()
                })
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
    fn abspath(&self) -> io::Result<PathBuf> {
        if self.as_ref().is_absolute() {
            return Ok(self.as_ref().to_owned());
        }

        Ok(env::current_dir()?.join(self))
    }
}

impl PathExt for Path {}
impl PathExt for PathBuf {}

/// Error when conversion from [`OsStr`] to [`String`] fails.
///
/// The contained [`String`] is is the lossy conversion of the original.
#[derive(Debug, thiserror::Error)]
#[error("failed to convert OsStr '{0}' to String")]
pub struct Utf8ConvError(pub String);

pub trait OsStrExt: AsRef<OsStr> {
    /// Try to convert this [`OsStr`] into a string.
    fn try_to_str(&self) -> Result<&str, Utf8ConvError> {
        match self.as_ref().to_str() {
            Some(s) => Ok(s),
            _ => Err(Utf8ConvError(self.as_ref().to_string_lossy().to_string())),
        }
    }
}

impl OsStrExt for OsStr {}
impl OsStrExt for std::ffi::OsString {}
impl OsStrExt for Path {}
impl OsStrExt for PathBuf {}

#[cfg(feature = "ureq")]
pub fn download_file_to(url: &str, writer: &mut impl std::io::Write) -> Result<()> {
    let req = ureq::get(url).call()?;
    if req.status() != 200 {
        anyhow::bail!(
            "Server at url '{}' returned unexpected status {}: {}",
            url,
            req.status(),
            req.status_text()
        );
    }

    let mut reader = req.into_reader();
    std::io::copy(&mut reader, writer)?;
    Ok(())
}
