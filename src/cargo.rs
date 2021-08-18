use std::ffi::OsStr;
use std::fmt::Write;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::*;
use cargo_toml::{Manifest, Product};
use log::*;

use crate::cmd;
use crate::utils::OsStrExt;

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum CargoCmd {
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

    /// Create a new crate using with the given `args`.
    ///
    /// Uses `cargo init` if `init` is true, otherwise uses `cargo new`.
    pub fn create(
        &self,
        init: bool,
        args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<()> {
        debug!("Generating new Cargo crate in path {}", self.0.display());
        cmd!(
            "cargo", if init { "init" } else { "new" };
            args=(args),
            arg=(&self.0)
        )?;
        Ok(())
    }

    /// Set the library type to `lib_type` and return its name.
    pub(crate) fn set_library_type(
        &self,
        lib_type: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<String> {
        let mut cargo_toml = self.load_manifest()?;
        let lib_type: Vec<_> = lib_type.into_iter().map(Into::into).collect();

        let name = self.get_lib_name(&cargo_toml);
        debug!(
            "Setting Cargo library crate {} to type {:?}",
            name, &lib_type
        );

        cargo_toml.lib = Some(Product {
            crate_type: Some(lib_type),
            ..if let Some(p) = cargo_toml.lib.take() {
                p
            } else {
                Default::default()
            }
        });

        self.save_manifest(&cargo_toml)?;

        Ok(name)
    }

    pub(crate) fn check_staticlib(&self) -> Result<String> {
        debug!("Checking Cargo.toml in {}", self.0.display());

        let cargo_toml = self.load_manifest()?;

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

    /// Create a `config.toml` in `.cargo` with a `[build]` and `[unstable]` section.
    pub fn create_config_toml(
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
            write!(
                &mut data,
                r#"[build]
target = "{}"
"#,
                target.as_ref()
            )?;
        }

        if build_std != BuildStd::None {
            write!(
                &mut data,
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
            )?;
        }

        fs::create_dir_all(cargo_config_toml_path.parent().unwrap())?;
        fs::write(cargo_config_toml_path, data)?;

        Ok(())
    }

    /// Load the manifest of this crate.
    pub fn load_manifest(&self) -> Result<Manifest> {
        Ok(Manifest::from_path(&self.0.join("Cargo.toml"))?)
    }

    /// Save the manifest of this crate.
    pub fn save_manifest(&self, toml: &Manifest) -> Result<()> {
        Ok(fs::write(
            &self.0.join("Cargo.toml"),
            toml::to_string(&toml)?,
        )?)
    }

    /// Load the cargo config of this crate.
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
            if let Some(value) = value {
                let result = f(value);

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

/// Set metadata that gets passed to all dependent's build scripts.
pub fn set_links_metadata(key: impl AsRef<str>, value: impl AsRef<str>) {
    println!("cargo:{}={}", key.as_ref(), value.as_ref());
}

/// Add an argument the cargo passes to the linker invocation for this package.
pub fn add_link_arg(arg: impl AsRef<str>) {
    println!("cargo:rustc-link-arg={}", arg.as_ref());
}

pub fn rerun_if_changed(file_or_dir: impl AsRef<Path>) {
    println!(
        "cargo:rerun-if-changed={}",
        file_or_dir.as_ref().try_to_str().unwrap()
    )
}

pub fn rerun_if_env_changed(env_var_name: impl AsRef<str>) {
    println!("cargo:rerun-if-env-changed={}", env_var_name.as_ref());
}

pub fn set_rustc_cfg(key: impl AsRef<str>, value: Option<impl AsRef<str>>) {
    if let Some(value) = value {
        println!("cargo:rustc-cfg={}={}", key.as_ref(), value.as_ref());
    } else {
        println!("cargo:rustc-cfg={}", key.as_ref());
    }
}

pub fn set_rustc_env(key: impl AsRef<str>, value: impl AsRef<str>) {
    println!("cargo:rustc-env={}={}", key.as_ref(), value.as_ref());
}
