use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use cargo_toml::{Manifest, Product};

use anyhow::*;
use log::*;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum Cargo {
    New(BuildStd),
    Init(BuildStd),
    Upgrade,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum BuildStd {
    None,
    Core,
    Std,
}

#[derive(Clone, Debug)]
pub struct Crate(PathBuf);

impl Crate {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self(dir.as_ref().to_owned())
    }

    pub fn create(&self, init: bool, options: &[impl AsRef<OsStr>]) -> Result<()> {
        debug!(
            "Generating new Cargo library crate in path {}",
            self.0.display()
        );

        let mut cmd = Command::new("cargo");

        cmd.arg(if init { "init" } else { "new" })
            .arg("--lib")
            .arg("--vcs")
            .arg("none") // TODO: For now, because otherwise espidf's CMake-based build fails
            .arg(&self.0)
            .args(options);

        debug!("Running command {:?}", cmd);

        cmd.status()?;

        Ok(())
    }

    pub(crate) fn update_staticlib(&self) -> Result<String> {
        let mut cargo_toml = self.load_toml()?;

        let name = self.get_lib_name(&cargo_toml);

        debug!("Setting Cargo library crate {} to type \"staticlib\"", name);

        cargo_toml.lib = Some(Product {
            crate_type: Some(vec!["staticlib".into()]),
            ..Default::default()
        });

        self.save_toml(&cargo_toml)?;

        Ok(name)
    }

    pub(crate) fn check_staticlib(&self) -> Result<String> {
        debug!("Checking Cargo.toml in {}", self.0.display());

        let cargo_toml = self.load_toml()?;

        if let Some(lib) = cargo_toml.lib.as_ref() {
            let empty_vec = &Vec::new();
            let crate_type = lib.crate_type.as_ref().unwrap_or(empty_vec);

            if crate_type
                .into_iter()
                .find(|s| s.as_str() == "staticlib")
                .is_some()
            {
                Ok(self.get_lib_name(&cargo_toml))
            } else {
                bail!("This library crate is missing a crate_type = [\"staticlib\"] declaration");
            }
        } else {
            bail!("Not a library crate");
        }
    }

    pub(crate) fn create_config_toml(
        &self,
        target: Option<impl AsRef<str>>,
        build_std: BuildStd,
    ) -> Result<()> {
        let cargo_config_toml_path = self.0.join(".cargo").join("config.toml");

        debug!(
            "Creating a Cargo config {}",
            cargo_config_toml_path.display()
        );

        let mut data = String::new();

        if let Some(target) = target {
            data.push_str(
                format!(
                    r#"[build]
target = "{}"
"#,
                    target.as_ref()
                )
                .as_str(),
            );
        }

        if build_std != BuildStd::None {
            data.push_str(
                format!(
                    r#"
[unstable]
build-std = ["{}", "panic_abort"]
build-std-features = ["panic_immediate_abort"]
"#,
                    if build_std == BuildStd::Std {
                        "std"
                    } else {
                        "core"
                    }
                )
                .as_str(),
            );
        }

        fs::create_dir_all(cargo_config_toml_path.parent().unwrap())?;
        fs::write(cargo_config_toml_path, data)?;

        Ok(())
    }

    pub fn load_toml(&self) -> Result<Manifest> {
        Ok(Manifest::from_path(&self.0.join("Cargo.toml"))?)
    }

    pub fn save_toml(&self, toml: &Manifest) -> Result<()> {
        Ok(fs::write(
            &self.0.join("Cargo.toml"),
            toml::to_string(&toml)?,
        )?)
    }

    pub fn load_config_toml(path: impl AsRef<Path>) -> Result<Option<toml::Value>> {
        let path = path.as_ref();

        let config = path.join(".cargo").join("config.toml");

        let config = if !config.exists() || !config.is_file() {
            path.join(".cargo").join("config")
        } else {
            config
        };

        Ok(if config.exists() && config.is_file() {
            let value = fs::read_to_string(&config)?.parse::<toml::Value>()?;

            info!("Found pre-configured {} in {}", value, config.display());

            Some(value)
        } else {
            None
        })
    }

    pub fn find_config_toml(&self) -> Result<Option<toml::Value>> {
        self.scan_config_toml(|value| Some(value))
    }

    pub fn scan_config_toml<F, Q>(&self, f: F) -> Result<Option<Q>>
    where
        F: Fn(toml::Value) -> Option<Q>,
    {
        let mut path = self.0.as_path();

        loop {
            let value = Self::load_config_toml(path)?;
            if value.is_some() {
                let result = f(value.unwrap());

                if result.is_some() {
                    return Ok(result);
                }
            }

            if let Some(parent_path) = path.parent() {
                path = parent_path;
            } else {
                break;
            }
        }

        Ok(None)
    }

    pub(crate) fn get_lib_name(&self, cargo_toml: &Manifest) -> String {
        let name_from_dir = self.0.file_name().unwrap().to_str().unwrap().to_owned();

        cargo_toml
            .lib
            .as_ref()
            .map(|lib| lib.name.clone())
            .flatten()
            .unwrap_or(
                cargo_toml
                    .package
                    .as_ref()
                    .map(|package| package.name.clone())
                    .unwrap_or(name_from_dir),
            )
            .replace('-', "_")
    }
}

pub mod build {
    use std::{
        convert::TryFrom,
        env,
        path::{Path, PathBuf},
        vec,
    };

    use anyhow::*;

    use crate::project::SconsVariables;

    const VAR_C_INCLUDE_ARGS_KEY: &'static str = "CARGO_PIO_C_INCLUDE_ARGS";
    const VAR_LINK_ARGS_KEY: &'static str = "CARGO_PIO_LINK_ARGS";

    pub const CARGO_PIO_LINK: &'static str = "cargo-pio-link";
    pub const CARGO_PIO_LINK_ARG_PREFIX: &'static str = "--cargo-pio-link-";
    pub const CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX: &'static str = "--cargo-pio-link-linker=";
    pub const CARGO_PIO_LINK_REMOVE_DUPLICATE_LIBS_ARG: &'static str =
        "--cargo-pio-link-remove-duplicate-libs";

    pub fn env_options_iter(
        env_var_prefix: impl AsRef<str>,
    ) -> Result<impl Iterator<Item = (String, String)>> {
        let env_var_prefix = env_var_prefix.as_ref().to_owned();

        Ok(env::vars()
            .filter(move |(key, _)| key.starts_with(format!("{}_", env_var_prefix).as_str()))
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
                key.starts_with(format!("{}_", env_var_prefix).as_str())
                    && key.ends_with(BASE_SUFFIX)
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
        println!("cargo:rerun-if-changed={}", to_string(path)?);

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

    pub fn to_string(path: impl AsRef<Path>) -> Result<String> {
        path.as_ref()
            .to_str()
            .ok_or(Error::msg("Cannot convert to str"))
            .map(str::to_owned)
    }

    fn split(arg: impl AsRef<str>) -> Vec<String> {
        arg.as_ref()
            .split(" ")
            .map(str::to_owned)
            .collect::<Vec<String>>()
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
            output(VAR_C_INCLUDE_ARGS_KEY, self.0.as_str());
        }
    }

    #[derive(Clone, Debug)]
    pub struct LinkArgs {
        libflags: vec::Vec<String>,
        linkflags: vec::Vec<String>,
        libdirflags: vec::Vec<String>,
        linker: PathBuf,
    }

    impl TryFrom<&SconsVariables> for LinkArgs {
        type Error = anyhow::Error;

        fn try_from(scons: &SconsVariables) -> Result<Self> {
            Ok(Self {
                libflags: split(&scons.libflags),
                linkflags: split(&scons.linkflags),
                libdirflags: split(&scons.libdirflags),
                linker: scons.full_path(&scons.link)?,
            })
        }
    }

    impl LinkArgs {
        pub fn output(&self, project_path: impl AsRef<Path>, remove_duplicate_libs: bool) {
            for arg in self.gather(project_path, remove_duplicate_libs) {
                output_link_arg(arg);
            }
        }

        pub fn propagate(&self, project_path: impl AsRef<Path>, remove_duplicate_libs: bool) {
            let args = self.gather(project_path, remove_duplicate_libs);

            output(VAR_LINK_ARGS_KEY, args.join(" "));
        }

        pub fn output_propagated(from_crate: impl AsRef<str>) -> Result<()> {
            for arg in split(env::var(format!(
                "DEP_{}_{}",
                from_crate.as_ref(),
                VAR_LINK_ARGS_KEY
            ))?) {
                output_link_arg(arg);
            }

            Ok(())
        }

        pub fn gather(
            &self,
            project_path: impl AsRef<Path>,
            remove_duplicate_libs: bool,
        ) -> vec::Vec<String> {
            let mut result = Vec::new();

            if Self::is_wrapped_linker() {
                result.push(format!(
                    "{}{}",
                    CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX,
                    self.linker.display()
                ));

                if remove_duplicate_libs {
                    result.push(CARGO_PIO_LINK_REMOVE_DUPLICATE_LIBS_ARG.to_owned());
                }
            }

            // A hack to workaround this issue with Rust's compiler intrinsics: https://github.com/rust-lang/compiler-builtins/issues/353
            //result.push("-Wl,--allow-multiple-definition".to_owned());

            result.push(format!("-L{}", project_path.as_ref().display().to_string()));

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

            result
        }

        fn is_wrapped_linker() -> bool {
            match env::var("RUSTC_LINKER").ok().as_ref().map(String::as_str) {
                Some(CARGO_PIO_LINK) => true,
                _ => false,
            }
        }
    }
}
