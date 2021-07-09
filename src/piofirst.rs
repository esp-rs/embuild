use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
};

use anyhow::*;
use log::*;

use cargo_toml::{Manifest, Product};
use toml;

use super::*;

pub const VAR_BUILD_ACTIVE: &'static str = "CARGO_PIO_BUILD_ACTIVE";
pub const VAR_BUILD_BINDGEN_RUN: &'static str = "CARGO_PIO_BUILD_BINDGEN_RUN";
pub const VAR_BUILD_PATH: &'static str = "CARGO_PIO_PATH";
pub const VAR_BUILD_INC_FLAGS: &'static str = "CARGO_PIO_BUILD_INC_FLAGS";
pub const VAR_BUILD_LIB_FLAGS: &'static str = "CARGO_PIO_BUILD_LIB_FLAGS";
pub const VAR_BUILD_LIB_DIR_FLAGS: &'static str = "CARGO_PIO_BUILD_LIB_DIR_FLAGS";
pub const VAR_BUILD_LIBS: &'static str = "CARGO_PIO_BUILD_LIBS";
pub const VAR_BUILD_LINK_FLAGS: &'static str = "CARGO_PIO_BUILD_LINK_FLAGS";
pub const VAR_BUILD_LINK: &'static str = "CARGO_PIO_BUILD_LINK";
pub const VAR_BUILD_LINKCOM: &'static str = "CARGO_PIO_BUILD_LINKCOM";
pub const VAR_BUILD_MCU: &'static str = "CARGO_PIO_BUILD_MCU";
pub const VAR_BUILD_BINDGEN_EXTRA_CLANG_ARGS: &'static str =
    "CARGO_PIO_BUILD_BINDGEN_EXTRA_CLANG_ARGS";

const PLATFORMIO_GIT_PY: &'static [u8] = include_bytes!("platformio.git.py.template");
const PLATFORMIO_PATCH_PY: &'static [u8] = include_bytes!("platformio.patch.py.template");
const PLATFORMIO_CARGO_PY: &'static [u8] = include_bytes!("platformio.cargo.py.template");

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum BuildStd {
    None,
    Core,
    Std,
}

pub fn regenerate_project(
    create_dir: bool,
    create_crate: bool,
    create_pioini: bool,
    path: impl AsRef<Path>,
    path_opt: Option<impl AsRef<Path>>,
    resolution: Option<Resolution>,
    build_std: BuildStd,
    cargo_args: Vec<String>,
) -> Result<()> {
    let rust_lib = if create_crate {
        let resolution = resolution.as_ref().unwrap();
        let rust_lib = generate_crate(create_dir, &path, path_opt, cargo_args)?;

        create_cargo_settings(&path, build_std, Some(resolution.target.as_str()))?;
        create_entry_points(&path)?;
        create_dummy_c_file(&path)?;

        rust_lib
    } else {
        check_crate(&path)?
    };

    if create_pioini {
        let resolution = resolution.as_ref().unwrap();

        create_platformio_ini(&path, rust_lib, resolution.target.as_str(), resolution)?;

        update_gitignore(&path)?;
    }

    create_platformio_git_py(&path)?;
    create_platformio_patch_py(&path)?;
    create_platformio_cargo_py(&path)?;

    Ok(())
}

fn generate_crate(
    new: bool,
    path: impl AsRef<Path>,
    path_opt: Option<impl AsRef<Path>>,
    cargo_args: Vec<String>,
) -> Result<String> {
    debug!(
        "Generating new Cargo library crate in path {}",
        path.as_ref().display()
    );

    let mut cmd = Command::new("cargo");

    cmd.arg(if new { "new" } else { "init" })
        .arg("--lib")
        .arg("--vcs")
        .arg("none") // TODO: For now, because otherwise espidf's CMake-based build fails
        .args(cargo_args);

    if let Some(path) = path_opt {
        cmd.arg(path.as_ref());
    }

    debug!("Running command {:?}", cmd);

    cmd.status()?;

    let cargo_toml_path = path.as_ref().join("Cargo.toml");

    let mut cargo_toml = Manifest::from_path(&cargo_toml_path)?;

    let name = get_lib_name(&cargo_toml_path, &cargo_toml);

    debug!("Setting Cargo library crate {} to type \"staticlib\"", name);

    cargo_toml.lib = Some(Product {
        crate_type: Some(vec!["staticlib".into()]),
        ..Default::default()
    });

    fs::write(cargo_toml_path, toml::to_string(&cargo_toml)?)?;

    Ok(name)
}

fn check_crate(path: impl AsRef<Path>) -> Result<String> {
    let cargo_toml_path = path.as_ref().join("Cargo.toml");
    debug!("Checking file {}", cargo_toml_path.display());

    let cargo_toml = Manifest::from_path(&cargo_toml_path)?;

    if let Some(lib) = cargo_toml.lib.as_ref() {
        let empty_vec = &Vec::new();
        let crate_type = lib.crate_type.as_ref().unwrap_or(empty_vec);

        if crate_type
            .into_iter()
            .find(|s| s.as_str() == "staticlib")
            .is_some()
        {
            Ok(get_lib_name(&cargo_toml_path, &cargo_toml))
        } else {
            bail!("This library crate is missing a crate_type = [\"staticlib\"] declaration");
        }
    } else {
        bail!("Not a library crate");
    }
}

fn get_lib_name(cargo_toml_path: impl AsRef<Path>, cargo_toml: &Manifest) -> String {
    let name_from_dir = cargo_toml_path
        .as_ref()
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();

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

fn create_platformio_ini(
    path: impl AsRef<Path>,
    rust_lib: impl AsRef<str>,
    rust_target: impl AsRef<str>,
    resolution: &Resolution,
) -> Result<()> {
    let platformio_ini_path = path.as_ref().join("platformio.ini");

    debug!(
        "Creating file {} with resolved params {:?}",
        platformio_ini_path.display(),
        resolution
    );

    fs::write(
        platformio_ini_path,
        format!(r#"
; PlatformIO Project Configuration File
;
; Please visit documentation for options and examples
; https://docs.platformio.org/page/projectconf.html
[platformio]
default_envs = debug

[env]
extra_scripts = pre:platformio.git.py, pre:platformio.patch.py, platformio.cargo.py
rust_lib = {}
rust_target = {}
board = {}
platform = {}
framework = {}
; Uncomment the line below if your platform does not have pre-built Rust core (e.g. ESP32)
;cargo_options = -Zbuild-std=core
; Uncomment the line below (and comment the line above) if your platform supports Rust std, but it is not pre-built (e.g. ESP32)
; If using Rust std, don't forget to comment out the #[panic_habdler] line in src/lib.rs
cargo_options = -Zbuild-std=std,panic_abort -Zbuild-std-features=panic_immediate_abort

[env:debug]
build_type = debug

[env:release]
build_type = release
"#,
        rust_lib.as_ref(),
        rust_target.as_ref(),
        resolution.board,
        resolution.platform,
        resolution.frameworks.join(", ")).as_bytes())?;

    Ok(())
}

fn create_entry_points(path: impl AsRef<Path>) -> Result<()> {
    let lib_rs_path = path.as_ref().join("src").join("lib.rs");

    debug!(
        "Creating a Rust library entry-point file {} with default entry points for various SDKs",
        lib_rs_path.display()
    );

    let data = r#"
// Remove if STD is supported for your platform and you plan to use it
#![no_std]

// Remove if STD is supported for your platform and you plan to use it
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

//
// The functions below are just sample entry points so that there are no linkage errors
// Leave only the one corresponding to your vendor SDK framework
//

////////////////////////////////////////////////////////
// Arduino                                            //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn setup() {
}

#[no_mangle]
#[export_name = "loop"]
extern "C" fn arduino_loop() {
}

////////////////////////////////////////////////////////
// ESP-IDF                                            //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn app_main() {
}

////////////////////////////////////////////////////////
// All others                                         //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn main() -> i32 {
    0
}
"#;

    fs::create_dir_all(lib_rs_path.parent().unwrap())?;
    fs::write(lib_rs_path, data)?;

    Ok(())
}

fn create_cargo_settings(
    path: impl AsRef<Path>,
    build_std: BuildStd,
    target: Option<impl AsRef<str>>,
) -> Result<()> {
    let cargo_config_toml_path = path.as_ref().join(".cargo").join("config.toml");

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

fn create_dummy_c_file(path: impl AsRef<Path>) -> Result<()> {
    let dummy_c_file_path = path.as_ref().join("src").join("cargo.c");

    debug!(
        "Creating the PlatformIO->Cargo build trigger C file {}",
        dummy_c_file_path.display()
    );

    fs::write(
        dummy_c_file_path,
        r#"
/*
 * Cargo <-> PlatformIO helper C file (autogenerated by cargo-pio)
 * This file is intentionally empty. Please DO NOT change it or delete it!
 *
 * Two reasons why this file is necessary:
 * - PlatformIO complains if the src directory is empty with an error message
 *   'Nothing to build. Please put your source code files to '../src' folder'. So we have to provide at least one C/C++ source file
 * - The Cargo invocation is attached as a post-action to building this file. This is necessary, or else
 *   Cargo crates will not see the extra include directories of all libraries downloaded via the PlatformIO Library Manager
 */
"#,
    )?;

    Ok(())
}

fn update_gitignore(path: impl AsRef<Path>) -> Result<()> {
    // TODO: Only do this if not done already
    debug!("Adding \".pio\" and \"CMakeFiles\" directories to .gitignore");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(path.as_ref().join(".gitignore"))?;

    writeln!(file, ".pio\nCMakeFiles/")?;

    Ok(())
}

fn create_platformio_git_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.git.py");

    fs::write(path.as_ref().join("platformio.git.py"), PLATFORMIO_GIT_PY)?;

    Ok(())
}

fn create_platformio_patch_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.patch.py");

    fs::write(
        path.as_ref().join("platformio.patch.py"),
        PLATFORMIO_PATCH_PY,
    )?;

    Ok(())
}

fn create_platformio_cargo_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.cargo.py");

    fs::write(
        path.as_ref().join("platformio.cargo.py"),
        PLATFORMIO_CARGO_PY,
    )?;

    Ok(())
}
