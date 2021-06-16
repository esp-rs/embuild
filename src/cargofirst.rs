use std::{fs, mem, path::Path};

use anyhow::*;
use log::*;

use super::*;

const PLATFORMIO_DUMP_PY: &'static [u8] = include_bytes!("platformio.dump.py.template");

pub fn build_framework(
    pio: &Pio,
    project_path: impl AsRef<Path>,
    release: bool,
    resolution: &Resolution,
) -> Result<SconsVariables> {
    create_and_build_framework_project(pio, project_path, release, false/*quick dump*/, false/*dump_only*/, resolution)
}

pub fn output_link_args(
    project_path: impl AsRef<Path>,
    scons_vars: &SconsVariables,
) -> Result<()> {
    for mut arg in split(&scons_vars.libflags) {
        // Hack: convert the relative paths that Pio generates to absolute ones
        if arg.starts_with(".pio/") {
            arg = format!("{}/{}", project_path.as_ref().display(), arg);
        } else if arg.starts_with(".pio\\") {
            arg = format!("{}\\{}", project_path.as_ref().display(), arg);
        }

        println!("cargo:rustc-link-arg-bins={}", arg);
    }

    println!("cargo:rustc-link-search={}", project_path.as_ref().display());

    for arg in split(&scons_vars.libdirflags) {
        println!("cargo:rustc-link-arg-bins={}", arg);
    }

    for arg in split(&scons_vars.linkflags) {
        println!("cargo:rustc-link-arg-bins={}", arg);
    }

    Ok(())
}

pub fn get_framework_scons_vars(pio: &Pio, release: bool, quick: bool, resolution: &Resolution) -> Result<SconsVariables> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path().join("proj");

    create_and_build_framework_project(pio, project_path, release, quick, true/*dump_only*/, resolution)
}

fn create_and_build_framework_project(
    pio: &Pio,
    project_path: impl AsRef<Path>,
    release: bool,
    quick_dump: bool,
    dump_only: bool,
    resolution: &Resolution,
) -> Result<SconsVariables> {
    create_project(&project_path, resolution, quick_dump, dump_only)?;

    let mut cmd = pio.run_cmd();

    cmd
        .arg("-d")
        .arg(project_path.as_ref())
        .arg("-t")
        .arg(if release {"release"} else {"debug"});

    pio.exec(&mut cmd)?;

    SconsVariables::from_json(project_path)
}

pub fn create_project(
    path: impl AsRef<Path>,
    resolution: &Resolution,
    quick_dump: bool,
    dump_only: bool,
) -> Result<()> {
    let path = path.as_ref();

    //let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path)?;

    create_platformio_ini(path, resolution, quick_dump, dump_only)?;
    create_platformio_dump_py(path)?;
    create_c_entry_points(path)?;

    Ok(())
}

fn create_platformio_ini(
    path: impl AsRef<Path>,
    resolution: &Resolution,
    quick_dump: bool,
    dump_only: bool,
) -> Result<()> {
    let platformio_ini_path = path.as_ref().join("platformio.ini");

    debug!("Creating file {} with resolved params {:?}", platformio_ini_path.display(), resolution);

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
extra_scripts = platformio.dump.py
board = {}
platform = {}
framework = {}
quick_dump = {}
terminate_after_dump = {}

[env:debug]
build_type = debug

[env:release]
build_type = release
"#,
        resolution.board,
        resolution.platform,
        resolution.frameworks.join(", "),
        quick_dump,
        dump_only,
    ).as_bytes())?;

    Ok(())
}

fn create_c_entry_points(path: impl AsRef<Path>) -> Result<()> {
    let main_c_path = path.as_ref().join("src").join("main.c");

    debug!("Creating a C entry-point file {} with default entry points for various SDKs", main_c_path.display());

    let data = r#"
//
// The functions below are just sample entry points so that there are no linkage errors
// Leave only the one corresponding to your vendor SDK framework
//

////////////////////////////////////////////////////////
// Arduino                                            //
////////////////////////////////////////////////////////

void setup() {
}

void loop() {
}

////////////////////////////////////////////////////////
// ESP-IDF                                            //
////////////////////////////////////////////////////////

void app_main() {
}

////////////////////////////////////////////////////////
// All others                                         //
////////////////////////////////////////////////////////

int main() {
    return 0;
}
"#;

    fs::create_dir_all(main_c_path.parent().unwrap())?;
    fs::write(main_c_path, data)?;

    Ok(())
}

fn create_platformio_dump_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.dump.py");

    fs::write(path.as_ref().join("platformio.dump.py"), PLATFORMIO_DUMP_PY)?;

    Ok(())
}

fn split(arg: impl AsRef<str>) -> Vec<String> {
    arg.as_ref().split(" ").map(str::to_owned).collect::<Vec<String>>()
}
