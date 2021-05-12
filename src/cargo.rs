use std::{ffi::OsStr, fs::{self, OpenOptions}, io::Write, path::Path, process::Command};

use anyhow::*;
use log::*;

use cargo_toml::{Manifest, Product};
use toml;

use super::*;

const CARGO_PY: &'static [u8] = include_bytes!("Cargo.py.template");

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum BuildStd {
    None,
    Core,
    Std
}

pub fn generate_crate(
        new: bool,
        path: impl AsRef<Path>,
        path_opt: Option<impl AsRef<Path>>,
        cargo_args: Vec<String>) -> Result<String> {
    debug!("Generating new Cargo library crate in path {}", path.as_ref().display());

    let mut cmd = Command::new("cargo");

    cmd
        .arg(if new { "new" } else { "init" })
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

pub fn check_crate(path: impl AsRef<Path>) -> Result<String> {
    let cargo_toml_path = path.as_ref().join("Cargo.toml");
    debug!("Checking file {}", cargo_toml_path.display());

    let cargo_toml = Manifest::from_path(&cargo_toml_path)?;

    if let Some(lib) = cargo_toml.lib.as_ref() {
        let empty_vec = &Vec::new();
        let crate_type = lib.crate_type.as_ref().unwrap_or(empty_vec);

        if crate_type.into_iter().find(|s| s.as_str() == "staticlib").is_some() {
            Ok(get_lib_name(&cargo_toml_path, &cargo_toml))
        } else {
            bail!("This library crate is missing a crate_type = [\"staticlib\"] declaration");
        }
    } else {
        bail!("Not a library crate");
    }
}

fn get_lib_name(cargo_toml_path: impl AsRef<Path>, cargo_toml: &Manifest) -> String {
    let name_from_dir = cargo_toml_path.as_ref()
        .parent().unwrap()
        .file_name().unwrap()
        .to_str().unwrap()
        .to_owned();

    cargo_toml.lib.as_ref()
        .map(|lib| lib.name.clone())
        .flatten()
        .unwrap_or(cargo_toml.package.as_ref()
            .map(|package| package.name.clone())
            .unwrap_or(name_from_dir))
            .replace('-', "_")
}

pub fn resolve_platformio_ini(pio: Pio, params: ResolutionParams) -> Result<Resolution> {
    Resolver::new(pio).params(params).resolve()
}

pub fn create_platformio_ini(
        path: impl AsRef<Path>,
        rust_lib: impl AsRef<str>,
        rust_target: impl AsRef<str>,
        resolution: &Resolution) -> Result<()> {
    let platformio_ini_path = path.as_ref().join("platformio.ini");

    debug!("Creating file {} with resolved params {:?}", platformio_ini_path.display(), resolution);

    fs::write(
        platformio_ini_path,
        format!(r#"
; PlatformIO Project Configuration File
;
; Please visit documentation for options and examples
; https://docs.platformio.org/page/projectconf.html

[env]
extra_scripts = Cargo.py
rust_lib = {}
rust_target = {}
board = {}
platform = {}
framework = {}

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

pub fn create_entry_points(path: impl AsRef<Path>) -> Result<()> {
    let lib_rs_path = path.as_ref().join("src").join("lib.rs");

    debug!("Creating a Rust library entry-point file {} with default entry points for various SDKs", lib_rs_path.display());

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

pub fn create_cargo_settings(path: impl AsRef<Path>, build_std: BuildStd, target: Option<impl AsRef<str>>) -> Result<()> {
    let cargo_config_toml_path = path.as_ref().join(".cargo").join("config.toml");

    debug!("Creating a Cargo config {}", cargo_config_toml_path.display());

    let mut data = String::new();

    if let Some(target) = target {
        data.push_str(format!(r#"[build]
target = "{}"
"#,
            target.as_ref()).as_str());
    }

    if build_std != BuildStd::None {
        data.push_str(format!(r#"
[unstable]
build-std = ["{}", "panic_abort"]
build-std-features = ["panic_immediate_abort"]
"#,
        if build_std == BuildStd::Std {"std"} else {"core"}).as_str());
    }

    fs::create_dir_all(cargo_config_toml_path.parent().unwrap())?;
    fs::write(cargo_config_toml_path, data)?;

    Ok(())
}

pub fn create_dummy_c_file(path: impl AsRef<Path>) -> Result<()> {
    let dummy_c_file_path = path.as_ref().join("src").join("dummy.c");

    debug!("Creating a dummy C file {} so that PlatformIO build does not complain", dummy_c_file_path.display());

    fs::write(dummy_c_file_path,r#"
/*
 * This dummy file is necessary, or else PlatformIO build complains with an error
 * 'Nothing to build. Please put your source code files to '../src' folder'
*/
    "#)?;

    Ok(())
}

pub fn update_gitignore(path: impl AsRef<Path>) -> Result<()> { // TODO: Only do this if not done already
    debug!("Adding \".pio\" and \"CMakeFiles\" directories to .gitignore");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(path.as_ref().join(".gitignore"))?;

    writeln!(file, ".pio\nCMakeFiles/")?;

    Ok(())
}

pub fn create_cargo_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating Cargo.py");

    fs::write(path.as_ref().join("Cargo.py"), CARGO_PY)?;

    Ok(())
}

pub fn run_platformio<'a, 'b>(pio: Pio, args: &[impl AsRef<OsStr>]) -> Result<()> {
    let mut cmd = pio.cmd();

    cmd
        .arg("run")
        .args(args);

    debug!("Running PlatformIO: {:?}", cmd);

    cmd.status()?;

    Ok(())
}

pub fn install_platformio(pio_dir: Option<impl AsRef<Path>>, download: bool) -> Result<Pio> {
    let mut pio_installer = if download { PioInstaller::new_download()? } else { PioInstaller::new()? };

    if let Some(pio_dir) = pio_dir {
        let pio_dir = pio_dir.as_ref();

        if !pio_dir.exists() {
            fs::create_dir(&pio_dir)?;
        }

        pio_installer.pio(&pio_dir);
    }

    pio_installer.update()
}

pub fn get_platformio(pio_dir: Option<impl AsRef<Path>>, download: bool) -> Result<Pio> {
    let mut pio_installer = if download { PioInstaller::new_download()? } else { PioInstaller::new()? };

    if let Some(pio_dir) = pio_dir {
        pio_installer.pio(pio_dir.as_ref());
    }

    pio_installer.check()
}
