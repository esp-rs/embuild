use std::ffi::OsStr;
use std::fmt::{Display, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::Result;
#[cfg(feature = "manifest")]
use cargo_toml::{Manifest, Product};
use log::*;

use crate::utils::{OsStrExt, PathExt};
use crate::{cargo, cmd};

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

    /// Create a new crate with the given `args` that will be forwarded to cargo.
    ///
    /// Uses `cargo init` if `init` is `true`, otherwise uses `cargo new`.
    pub fn create(
        &self,
        init: bool,
        options: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<()> {
        debug!("Generating new Cargo crate in path {}", self.0.display());

        cmd!(
            "cargo", if init { "init" } else {"new"},
            @options,
            &self.0
        )?;
        Ok(())
    }

    /// Set the library type to `lib_type` and return its name.
    #[cfg(feature = "manifest")]
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

    /// Check that the library is a `staticlib` and return its name.
    #[cfg(feature = "manifest")]
    pub(crate) fn check_staticlib(&self) -> Result<String> {
        debug!("Checking Cargo.toml in {}", self.0.display());

        let cargo_toml = self.load_manifest()?;

        if let Some(ref lib) = cargo_toml.lib {
            let empty_vec = &Vec::new();
            let crate_type = lib.crate_type.as_ref().unwrap_or(empty_vec);

            if crate_type.iter().any(|s| s == "staticlib") {
                Ok(self.get_lib_name(&cargo_toml))
            } else {
                anyhow::bail!(
                    "This library crate is missing a crate_type = [\"staticlib\"] declaration"
                );
            }
        } else {
            anyhow::bail!("Not a library crate");
        }
    }

    /// Create a `config.toml` in `.cargo` with a `[target]` and `[unstable]` section.
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
    #[cfg(feature = "manifest")]
    pub fn load_manifest(&self) -> Result<Manifest> {
        Ok(Manifest::from_path(&self.0.join("Cargo.toml"))?)
    }

    /// Save the manifest of this crate.
    #[cfg(feature = "manifest")]
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
            info!("Found {}", config.display());

            Some(fs::read_to_string(&config)?.parse::<toml::Value>()?)
        } else {
            None
        })
    }

    pub fn find_config_toml(&self) -> Result<Option<toml::Value>> {
        self.scan_config_toml(Some)
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

    /// Get the library name from its manifest or directory name.
    #[cfg(feature = "manifest")]
    pub(crate) fn get_lib_name(&self, cargo_toml: &Manifest) -> String {
        let name_from_dir = self.0.file_name().unwrap().to_str().unwrap().to_owned();

        cargo_toml
            .lib
            .as_ref()
            .and_then(|lib| lib.name.clone())
            .unwrap_or_else(|| {
                cargo_toml
                    .package
                    .as_ref()
                    .map(|package| package.name.clone())
                    .unwrap_or(name_from_dir)
            })
            .replace('-', "_")
    }

    /// Get the path to a binary that is produced when building this crate.
    #[cfg(feature = "manifest")]
    pub fn get_binary_path<'a>(
        &self,
        release: bool,
        target: Option<&'a str>,
        binary: Option<&'a str>,
    ) -> Result<PathBuf> {
        let bin_products = self.load_manifest()?.bin;

        if bin_products.is_empty() {
            anyhow::bail!("Not a binary crate");
        }

        let bin_product = if let Some(binary) = binary {
            bin_products
                .iter()
                .find(|p| match p.name.as_ref() {
                    Some(b) => b == binary,
                    _ => false,
                })
                .ok_or_else(|| anyhow::anyhow!("Cannot locate binary with name {}", binary))?
        } else {
            if bin_products.len() > 1 {
                anyhow::bail!(
                    "This crate defines multiple binaries ({:?}), please specify binary name",
                    bin_products
                );
            }

            &bin_products[0]
        };

        let mut path = self.0.join("target");

        if let Some(target) = target {
            path = path.join(target)
        }

        Ok(path
            .join(if release { "release" } else { "debug" })
            .join(bin_product.name.as_ref().unwrap()))
    }

    /// Get the default target that would be used when building this crate.
    pub fn get_default_target(&self) -> Result<Option<String>> {
        self.scan_config_toml(|value| {
            value
                .get("build")
                .and_then(|table| table.get("target"))
                .and_then(|value| value.as_str())
                .map(|str| str.to_owned())
        })
    }
}

/// Set metadata that gets passed to all dependent's build scripts.
///
/// All dependent packages of this crate can gets the metadata set here in their build
/// script from an environment variable named `CARGO_DEP_<links value>_<key>`. The `<links
/// value>` is the value of the `links` property in this crate's manifest.
pub fn set_metadata(key: impl Display, value: impl Display) {
    println!("cargo:{}={}", key, value);
}

/// Add an argument that cargo passes to the linker invocation for this package.
pub fn add_link_arg(arg: impl Display) {
    println!("cargo:rustc-link-arg={}", arg);
}

/// Rerun this build script if the file or directory has changed.
pub fn track_file(file_or_dir: impl AsRef<Path>) {
    println!(
        "cargo:rerun-if-changed={}",
        file_or_dir.as_ref().try_to_str().unwrap()
    )
}

/// Rerun this build script if the environment variable has changed.
pub fn track_env_var(env_var_name: impl Display) {
    println!("cargo:rerun-if-env-changed={}", env_var_name);
}

/// Set a cfg key value pair for this package wich may be used for conditional
/// compilation.
pub fn set_rustc_cfg(key: impl Display, value: impl AsRef<str>) {
    if value.as_ref().is_empty() {
        println!("cargo:rustc-cfg={}", key);
    } else {
        println!(
            "cargo:rustc-cfg={}=\"{}\"",
            key,
            value.as_ref().replace('\"', "\\\"")
        );
    }
}

/// Set an environment variable that is available during this packages compilation.
pub fn set_rustc_env(key: impl Display, value: impl Display) {
    println!("cargo:rustc-env={}={}", key, value);
}

/// Display a warning on the terminal.
pub fn print_warning(warning: impl Display) {
    println!("cargo:warning={}", warning);
}

/// Get the out directory of a crate.
///
/// Panics if environment variable `OUT_DIR` is not set
/// (ie. when called outside of a build script).
pub fn out_dir() -> PathBuf {
    env::var_os("OUT_DIR")
        .expect("`OUT_DIR` env variable not set (maybe called outside of build script)")
        .into()
}

/// Extension trait for turning [`Display`]able values into cargo warnings.
pub trait IntoWarning<R> {
    /// Print as a cargo warning.
    ///
    /// This will `{:#}`-print all lines using `println!("cargo:warning={}", line)` where
    /// the first line's `Error:` prefix is removed and trimmed.
    fn into_warning(self) -> R;
}

impl<E> IntoWarning<()> for E
where
    E: Display,
{
    fn into_warning(self) {
        let fmt = format!("{:#}", self);
        let fmt = fmt.strip_prefix("Error:").unwrap_or(&fmt).trim_start();
        let mut lines = fmt.lines();

        let line = lines.next().unwrap_or("(empty)");
        cargo::print_warning(line);

        for line in lines {
            cargo::print_warning(line);
        }
    }
}

impl<V, E> IntoWarning<Option<V>> for Result<V, E>
where
    E: IntoWarning<()>,
{
    fn into_warning(self) -> Option<V> {
        match self {
            Ok(v) => Some(v),
            Err(e) => {
                e.into_warning();
                None
            }
        }
    }
}

/// Try to get the path to crate workspace dir or [`None`] if unavailable.
///
/// The crate workspace directory is the directory containing the cargo `target` dir in
/// which all artifacts for compilation are stored. We can only get the workspace
/// directory if we're currently running inside a cargo build script.
pub fn workspace_dir() -> Option<PathBuf> {
    // We pop the path to the out dir 6 times to get to the workspace root so the
    // directory containing the `target` (build) directory. The directory containing the
    // `target` directory is always the workspace root.

    // We have to pop one less if `$HOST == $TARGET` because then cargo will compile
    // directly into the `debug` or `release` directory instead of having that directory
    // inside of a `<target>` directory.
    let pop_count = if env::var_os("HOST")? == env::var_os("TARGET")? {
        5
    } else {
        6
    };
    Some(PathBuf::from(env::var_os("OUT_DIR")?).pop_times(pop_count))
}
