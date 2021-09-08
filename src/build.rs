use std::ffi::OsStr;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::{env, vec};

use anyhow::*;

use crate::cargo::{self, add_link_arg, print_warning, set_metadata, track_file};
use crate::cli::{self, Arg, ArgDef};
use crate::utils::OsStrExt;

const VAR_C_INCLUDE_ARGS: &str = "C_INCLUDE_ARGS";
const VAR_LINK_ARGS: &str = "LINK_ARGS";
const LINK_ARGS_FILE_NAME: &str = "linker_args.txt";

pub const LDPROXY_NAME: &str = "ldproxy";

pub const LDPROXY_LINKER_ARG: ArgDef = Arg::option("ldproxy-linker").long();
pub const LDPROXY_DEDUP_LIBS_ARG: ArgDef = Arg::flag("ldproxy-dedup-libs").long();
pub const LDPROXY_WORKING_DIRECTORY_ARG: ArgDef = Arg::option("ldproxy-cwd").long();

pub fn env_options_iter(
    env_var_prefix: impl AsRef<str>,
) -> Result<impl Iterator<Item = (String, String)>> {
    let env_var_prefix = env_var_prefix.as_ref().to_owned() + "_";

    Ok(env::vars()
        .filter(move |(key, _)| key.starts_with(&env_var_prefix))
        .map(|(_, value)| {
            let split = value.split('=').collect::<vec::Vec<_>>();

            (split[0].trim().to_owned(), split[1].trim().to_owned())
        }))
}

pub fn tracked_env_globs_iter(
    env_var_prefix: impl AsRef<str>,
) -> Result<impl Iterator<Item = (PathBuf, PathBuf)>> {
    track_sources(env_globs_iter(env_var_prefix)?)
}

pub fn env_globs_iter(
    env_var_prefix: impl AsRef<str>,
) -> Result<impl Iterator<Item = (PathBuf, PathBuf)>> {
    const BASE_SUFFIX: &str = "_BASE";

    let env_var_prefix = env_var_prefix.as_ref().to_owned();

    Ok(env::vars()
        .filter(move |(key, _)| {
            key.starts_with(format!("{}_", env_var_prefix).as_str()) && key.ends_with(BASE_SUFFIX)
        })
        .map(|(key, value)| {
            let base = PathBuf::from(value);
            let key_prefix = &key[0..key.len() - (BASE_SUFFIX.len() - 1)];

            let globs = env::vars()
                .filter(|(key, _)| key.starts_with(key_prefix) && !key.ends_with(BASE_SUFFIX))
                .map(|(_, value)| value)
                .collect::<vec::Vec<_>>();

            globs_iter(base, &globs)
        })
        .filter_map(Result::ok)
        .flatten())
}

pub fn tracked_globs_iter(
    base: impl AsRef<Path>,
    globs: &[impl AsRef<str>],
) -> Result<impl Iterator<Item = (PathBuf, PathBuf)>> {
    track_sources(globs_iter(base, globs)?)
}

pub fn globs_iter(
    base: impl AsRef<Path>,
    globs: &[impl AsRef<str>],
) -> Result<impl Iterator<Item = (PathBuf, PathBuf)>> {
    let base = base.as_ref().to_owned();

    Ok(globwalk::GlobWalkerBuilder::from_patterns(&base, globs)
        .follow_links(true)
        .build()?
        .into_iter()
        .filter_map(Result::ok)
        .map(move |entry| {
            entry
                .path()
                .strip_prefix(&base)
                .map(|dest| (entry.path().to_owned(), dest.to_owned()))
        })
        .filter_map(Result::ok))
}

pub fn track_sources<I, P>(iter: I) -> Result<impl Iterator<Item = (PathBuf, PathBuf)>>
where
    I: Iterator<Item = (P, P)>,
    P: AsRef<Path>,
{
    let items = iter
        .map(|(source, dest)| (source.as_ref().to_owned(), dest.as_ref().to_owned()))
        .collect::<vec::Vec<_>>();

    for (source, _) in &items {
        track_file(source);
    }

    Ok(items.into_iter())
}

#[derive(Clone, Debug)]
pub struct CInclArgs(pub String);

impl CInclArgs {
    pub fn propagate(&self) {
        set_metadata(VAR_C_INCLUDE_ARGS, self.0.as_str());
    }

    pub fn from_propagated(lib_name: impl Display) -> Result<CInclArgs> {
        Ok(CInclArgs(env::var(format!(
            "DEP_{}_{}",
            lib_name, VAR_C_INCLUDE_ARGS
        ))?))
    }
}

#[derive(Clone, Debug, Default)]
pub struct LinkArgsBuilder {
    pub libflags: Vec<String>,
    pub linkflags: Vec<String>,
    pub libdirflags: Vec<String>,
    pub(crate) force_ldproxy: bool,
    /// The path to the linker executable.
    pub(crate) linker: Option<PathBuf>,
    /// The working directory that should be set when linking.
    pub(crate) working_directory: Option<PathBuf>,
    pub(crate) dedup_libs: bool,
}

impl LinkArgsBuilder {
    pub fn force_ldproxy(mut self, value: bool) -> Self {
        self.force_ldproxy = value;
        self
    }

    pub fn linker(mut self, path: impl Into<PathBuf>) -> Self {
        self.linker = Some(path.into());
        self
    }

    pub fn working_directory(mut self, dir: impl AsRef<Path>) -> Self {
        self.working_directory = Some(dir.as_ref().to_owned());
        self
    }

    pub fn dedup_libs(mut self, dedup: bool) -> Self {
        self.dedup_libs = dedup;
        self
    }

    pub fn build(self) -> Result<LinkArgs> {
        let args: Vec<_> = self
            .libdirflags
            .into_iter()
            .chain(self.libflags)
            .chain(self.linkflags)
            .collect();

        let detected_ldproxy = env::var("RUSTC_LINKER")
            .ok()
            .and_then(|l| {
                Path::new(&l)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .map(|s| s == LDPROXY_NAME)
            })
            .unwrap_or(false);

        if self.force_ldproxy && !detected_ldproxy {
            print_warning(
                "The linker arguments force the usage of `ldproxy` but the linker used \
                 by cargo is different. Please set the linker to `ldproxy` in your cargo config \
                 or set `force_ldproxy` to `false`.",
            );
        }

        let result = if self.force_ldproxy || detected_ldproxy {
            let mut result = Vec::new();

            if let Some(linker) = &self.linker {
                result.extend(LDPROXY_LINKER_ARG.format(Some(linker.try_to_str()?)));
            }

            if self.dedup_libs {
                result.extend(LDPROXY_DEDUP_LIBS_ARG.format(None));
            }

            if let Some(cwd) = &self.working_directory {
                result.extend(LDPROXY_WORKING_DIRECTORY_ARG.format(Some(cwd.try_to_str()?)))
            }

            // If `windows && gcc` we always use reponse files to circumvent the command-line
            // length limitation.
            // TODO: implement other linkers
            if cfg!(windows) {
                // TODO: add way to detect linker flavor
                let is_gcc = self
                    .linker
                    .and_then(|l| {
                        l.file_stem()
                            .and_then(OsStr::to_str)
                            .map(|s| s.ends_with("gcc"))
                    })
                    .unwrap_or(false);

                if is_gcc {
                    let link_args_file = cargo::out_dir().join(LINK_ARGS_FILE_NAME);
                    let args = cli::join_unix_args(args.iter().map(|s| s.as_str()));

                    std::fs::write(&link_args_file, args).with_context(|| {
                        anyhow!(
                            "could not write link args to file '{}'",
                            link_args_file.display()
                        )
                    })?;

                    result.push(format!("@{}", link_args_file.try_to_str()?));
                } else {
                    result.extend(args);
                }

                result
            } else {
                result.extend(args);
                result
            }
        } else {
            args
        };

        Ok(LinkArgs { args: result })
    }
}

#[derive(Clone, Debug)]
pub struct LinkArgs {
    args: Vec<String>,
}

impl LinkArgs {
    /// Add the linker arguments from the native library.
    pub fn output(&self) {
        for arg in self.args.iter() {
            add_link_arg(arg);
        }
    }

    /// Propagate all linker arguments to all dependents of this crate.
    ///
    /// ### **Important**
    /// Calling this method in a dependency doesn't do anything on itself. All dependents
    /// that want to have these linker arguments propagated must call
    /// [`LinkArgs::output_propagated`] in their build script with the value of this
    /// crate's `links` property (specified in `Cargo.toml`).
    pub fn propagate(&self) {
        // TODO: maybe more efficient escape machanism
        set_metadata(
            VAR_LINK_ARGS,
            cli::join_unix_args(self.args.iter().map(|s| s.as_str())),
        );
    }

    /// Add all linker arguments from `lib_name` which have been propagated using [`propagate`](LinkArgs::propagate).
    ///
    /// `lib_name` doesn't refer to a crate, library or package name, it refers to a
    /// dependency's `links` property value, which is specified in its package manifest
    /// (`Cargo.toml`).
    pub fn output_propagated(lib_name: impl Display) -> Result<()> {
        let args = env::var(format!("DEP_{}_{}", lib_name, VAR_LINK_ARGS))?;

        for arg in cli::UnixCommandArgs::new(&args) {
            add_link_arg(arg);
        }

        Ok(())
    }
}
