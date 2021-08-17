use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::{env, vec};

use anyhow::*;

use crate::pio::project::SconsVariables;
use crate::utils::OsStrExt;

const VAR_C_INCLUDE_ARGS: &'static str = "C_INCLUDE_ARGS";
const VAR_LINK_ARGS: &'static str = "LINK_ARGS";
const LINK_ARGS_FILE: &'static str = "link-args.rsp";

pub const LINKPROXY_PREFIX: &'static str = "--linkproxy-";
pub const LINKPROXY_LINKER_ARG: &'static str = "--linkproxy-linker";
pub const LINKPROXY_DEDUP_LIBS_ARG: &'static str = "--linkproxy-dedup-libs";

pub fn env_options_iter(
    env_var_prefix: impl AsRef<str>,
) -> Result<impl Iterator<Item = (String, String)>> {
    let env_var_prefix = env_var_prefix.as_ref().to_owned() + "_";

    Ok(env::vars()
        .filter(move |(key, _)| key.starts_with(&env_var_prefix))
        .map(|(_, value)| {
            let split = value.split("=").collect::<vec::Vec<_>>();

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
    const BASE_SUFFIX: &'static str = "_BASE";

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

pub fn track(path: impl AsRef<Path>) -> Result<()> {
    println!("cargo:rerun-if-changed={}", path.as_ref().try_to_str()?);

    Ok(())
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
        track(source)?;
    }

    Ok(items.into_iter())
}

pub fn output(key: impl AsRef<str>, value: impl AsRef<str>) {
    println!("cargo:{}={}", key.as_ref(), value.as_ref());
}

pub fn output_link_arg(arg: impl AsRef<str>) {
    println!("cargo:rustc-link-arg={}", arg.as_ref());
}

#[derive(Clone, Debug)]
pub struct CInclArgs(String);

impl TryFrom<&SconsVariables> for CInclArgs {
    type Error = anyhow::Error;

    fn try_from(scons: &SconsVariables) -> Result<Self> {
        Ok(Self(scons.incflags.clone()))
    }
}

impl CInclArgs {
    pub fn propagate(&self) {
        output(VAR_C_INCLUDE_ARGS, self.0.as_str());
    }
}

#[derive(Clone, Debug, Default)]
pub struct LinkArgsBuilder {
    pub libflags: Vec<String>,
    pub linkflags: Vec<String>,
    pub libdirflags: Vec<String>,
    pub(crate) use_linkproxy: bool,
    /// The path to the linker executable.
    pub(crate) linker: Option<PathBuf>,
    /// The working directory that should be set when linking.
    pub(crate) working_directory: Option<PathBuf>,
    pub(crate) dedup_libs: bool,
}

impl LinkArgsBuilder {
    pub fn use_linkproxy(&mut self, value: bool) -> &mut Self {
        self.use_linkproxy = value;
        self
    }

    pub fn working_directory(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.working_directory = Some(dir.as_ref().to_owned());
        self
    }

    pub fn dedup_libs(&mut self, dedup: bool) -> &mut Self {
        self.dedup_libs = dedup;
        self
    }

    pub fn build(&mut self, project_path: impl AsRef<Path>) -> LinkArgs {
        let mut result = Vec::new();

        if self.use_linkproxy {
            if let Some(linker) = &self.linker {
                result.push(format!(
                    "{}{}",
                    LINKPROXY_LINKER_ARG,
                    linker.display()
                ));
            }

            if self.dedup_libs {
                result.push(LINKPROXY_DEDUP_LIBS_ARG.into());
            }
        }

        // A hack to workaround this issue with Rust's compiler intrinsics: https://github.com/rust-lang/compiler-builtins/issues/353
        //result.push("-Wl,--allow-multiple-definition".to_owned());

        result.push(format!("-L{}", project_path.as_ref().display()));

        for arg in &self.libdirflags {
            result.push(arg.clone());
        }

        for arg in &self.libflags {
            // Hack: convert the relative paths that Pio generates to absolute ones
            let arg = if arg.starts_with(".pio/") {
                format!("{}/{}", project_path.as_ref().display(), arg)
            } else if arg.starts_with(".pio\\") {
                format!("{}\\{}", project_path.as_ref().display(), arg)
            } else {
                arg.clone()
            };

            result.push(arg);
        }

        for arg in &self.linkflags {
            result.push(arg.clone());
        }

        LinkArgs { args: result }
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
            output_link_arg(arg);
        }
    }

    /// Propagate all linker arguments to all dependents of this crate.
    ///
    /// ### **Important**
    /// Calling this method in a dependency doesn't do anything on itself. All dependents
    /// that want to have these linker arguments propagated must call
    /// [`LinkerArgs::output_propagated`] in their build script with the value of this
    /// crate's `links` property (specified in `Cargo.toml`).
    pub fn propagate(&self) {
        output(VAR_LINK_ARGS, self.args.join("\n"));
    }

    /// Add all linker arguments from `lib_name` which have been propagated using [`propagate`].
    ///
    /// `lib_name` doesn't refer to a crate, library or package name, it refers to a
    /// dependency's `links` property value, which is specified in its package manifest
    /// (`Cargo.toml`).
    pub fn output_propagated(lib_name: impl AsRef<str>) -> Result<()> {
        for arg in env::var(format!("DEP_{}_{}", lib_name.as_ref(), VAR_LINK_ARGS))?.lines() {
            output_link_arg(arg);
        }

        Ok(())
    }
}
