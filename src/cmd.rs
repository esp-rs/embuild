//! Command building and running utilities.

use std::ffi::OsStr;
use std::io;
use std::process::{self, Command, ExitStatus};

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

/// A wrapper over a [`std::process::Command`] with more features.
#[derive(Debug)]
pub struct Cmd {
    /// The actual [`std::process::Command`] wrapped.
    pub cmd: std::process::Command,
    ignore_exitcode: bool,
}

impl std::ops::Deref for Cmd {
    type Target = std::process::Command;

    fn deref(&self) -> &Self::Target {
        &self.cmd
    }
}

impl std::ops::DerefMut for Cmd {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cmd
    }
}

impl From<std::process::Command> for Cmd {
    fn from(cmd: std::process::Command) -> Self {
        Cmd {
            cmd,
            ignore_exitcode: false,
        }
    }
}

impl From<Cmd> for std::process::Command {
    fn from(cmd: Cmd) -> Self {
        cmd.into_inner()
    }
}

impl Cmd {
    /// Construct a new [`Cmd`] for launching `program` (see
    /// [`std::process::Command::new`]).
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            cmd: Command::new(program),
            ignore_exitcode: false,
        }
    }

    /// Ignore the exit code when executing this command.
    ///
    /// Applies to:
    /// - [`Cmd::run`]
    /// - [`Cmd::output`]
    /// - [`Cmd::stdout`]
    /// - [`Cmd::stderr`]
    pub fn ignore_exitcode(&mut self) -> &mut Self {
        self.ignore_exitcode = true;
        self
    }

    /// Run the command to completion.
    ///
    /// If [`Cmd::ignore_exitcode`] has been called a program that exited with an error
    /// will also return [`Ok`], otherwise it will return [`Err`].
    /// A program that failed to start will always return an [`Err`].
    ///
    /// [`std::process::Command::status`] is used internally.
    pub fn run(&mut self) -> Result<(), CmdError> {
        self.cmd
            .status()
            .map_err(|e| CmdError::no_run(&self.cmd, e))
            .and_then(|v| {
                if self.ignore_exitcode {
                    Ok(())
                } else {
                    CmdError::status_into_result(v, &self.cmd, || None)
                }
            })
    }

    /// Run the command and get its [`ExitStatus`].
    pub fn status(&mut self) -> Result<ExitStatus, CmdError> {
        self.cmd
            .status()
            .map_err(|e| CmdError::no_run(&self.cmd, e))
    }

    fn print_output(&self, output: &std::process::Output) {
        // TODO: add some way to quiet this output
        use std::io::Write;
        std::io::stdout().write_all(&output.stdout[..]).ok();
        std::io::stderr().write_all(&output.stderr[..]).ok();
    }

    /// Run the command to completion and use its [`std::process::Output`] with `func`.
    ///
    /// If [`Cmd::ignore_exitcode`] has been called a program that exited with an error
    /// will also return [`Ok`], otherwise it will return [`Err`].
    /// A program that failed to start will always return an [`Err`].
    ///
    /// [`std::process::Command::output`] is used internally.
    pub fn output<T>(
        &mut self,
        func: impl FnOnce(std::process::Output) -> T,
    ) -> Result<T, CmdError> {
        match self.cmd.output() {
            Err(err) => Err(CmdError::no_run(&self.cmd, err)),
            Ok(result) => if self.ignore_exitcode {
                self.print_output(&result);
                Ok(())
            } else {
                CmdError::status_into_result(result.status, &self.cmd, || {
                    Some(
                        String::from_utf8_lossy(&result.stderr[..])
                            .trim_end()
                            .to_string(),
                    )
                })
            }
            .map_err(|e| {
                self.print_output(&result);
                e
            })
            .map(|_| func(result)),
        }
    }

    /// Run the command to completion and get its stdout output.
    ///
    /// See [`Cmd::output`].
    pub fn stdout(&mut self) -> Result<String, CmdError> {
        self.output(|output| {
            String::from_utf8_lossy(&output.stdout[..])
                .trim_end()
                .to_string()
        })
    }

    /// Run the command to completion and get its stderr output.
    ///
    /// See [`Cmd::output`].
    pub fn stderr(&mut self) -> Result<String, CmdError> {
        self.output(|output| {
            String::from_utf8_lossy(&output.stderr[..])
                .trim_end()
                .to_string()
        })
    }

    /// Turn this [`Cmd`] into its underlying [`std::process::Command`].
    pub fn into_inner(self) -> std::process::Command {
        self.cmd
    }
}

/// Build a command using a given [`std::process::Command`] or [`Cmd`] and return it.
///
/// The first argument is expected to be a [`std::process::Command`] or [`Cmd`] instance.
///
/// For a `new` builder the second argument, the program to run (passed to
/// [`std::process::Command::new`]) is mandatory. Every comma seperated argument
/// thereafter is added to the command's arguments. Arguments after an `@`-sign specify
/// collections of arguments (specifically `impl IntoIterator<Item = impl AsRef<OsStr>`).
/// The opional `key=value` arguments after a semicolon are simply translated to calling
/// the `std::process::Command::<key>` method with `value` as its arguments.
///
/// **Note:**
/// `@`-arguments must be followed by at least one normal argument. For example
///  `cmd_build!(new, "cmd", @args)` will not compile but `cmd_build!(new, "cmd", @args,
/// "other")` will. You can use `key=value` arguments to work around this limitation:
/// `cmd_build!(new, "cmd"; args=(args))`.
///
/// At the end the built [`std::process::Command`] is returned.
///
/// # Examples
/// ```
/// # use embuild::{cmd::Cmd, cmd_build};
/// let args_list = ["--foo", "--bar", "value"];
/// let mut cmd = Cmd::new("git");
/// let mut cmd = cmd_build!(cmd, @args_list, "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd_build {
    ($builder:ident $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        $(
            $($builder .args($cmdargs);)*
            $builder .arg($cmdarg);
        )*
        $($($builder . $k $v;)*)?

        $builder
    }}
}

/// Create a new [`Cmd`] instance.
///
/// This is a simple wrapper over the [`std::process::Command`] and [`Cmd`] API. It
/// expects at least one argument for the program to run. Every comma seperated argument
/// thereafter is added to the command's arguments. Arguments after an `@`-sign specify
/// collections of arguments (specifically `impl IntoIterator<Item = impl AsRef<OsStr>`).
/// The opional `key=value` arguments after a semicolon are simply translated to calling
/// the `Cmd::<key>` method with `value` as its arguments.
///
/// **Note:**
///  `@`-arguments must be followed by at least one normal argument. For example
/// `cmd!("cmd", @args)` will not compile but `cmd!("cmd", @args, "other")` will. You can
/// use `key=value` arguments to work around this limitation: `cmd!("cmd"; args=(args))`.
///
/// # Examples
/// ```
/// # use embuild::cmd;
/// let args_list = ["--foo", "--bar", "value"];
/// let mut cmd = cmd!("git", @args_list, "clone"; arg=("url.com"), env=("var", "value"));
/// ```
#[macro_export]
macro_rules! cmd {
    ($cmd:expr $(, $(@$cmdargs:expr,)* $cmdarg:expr)* $(; $($k:ident = $v:tt),*)?) => {{
        let mut cmd = $crate::cmd::Cmd::new($cmd);
        $crate::cmd_build!(cmd $(, $(@$cmdargs,)* $cmdarg)* $(; $($k = $v),* )?)
    }};
}
