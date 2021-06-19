use std::{fs, path::Path};

use anyhow::*;
use log::*;

use globwalk;

use super::*;

const PLATFORMIO_DUMP_PY: &'static [u8] = include_bytes!("platformio.dump.py.template");
const PLATFORMIO_PATCH_PY: &'static [u8] = include_bytes!("platformio.patch.py.template");

pub fn build_framework(
    pio: &Pio,
    project_path: impl AsRef<Path>,
    release: bool,
    resolution: &Resolution,
    patches: &[(&Path, &Path)],
    env_var_pio_conf_prefix: Option<impl AsRef<str>>,
    env_var_file_copy_prefix: Option<impl AsRef<str>>,
) -> Result<SconsVariables> {
    create_project(&project_path, resolution, patches, env_var_pio_conf_prefix, false/*quick dump*/, false/*dump_only*/)?;

    copy_files(&project_path, env_var_file_copy_prefix)?;
    apply_patches(&project_path, patches)?;

    build_project(pio, &project_path, release)
}

pub fn get_framework_scons_vars(pio: &Pio, release: bool, quick: bool, resolution: &Resolution) -> Result<SconsVariables> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path().join("proj");

    create_project(
        &project_path,
        resolution,
        &[],
        Option::<&str>::None,
        quick,
        true/*dump_only*/)?;

    build_project(pio, &project_path, release)
}

pub fn create_project(
    path: impl AsRef<Path>,
    resolution: &Resolution,
    patches: &[(&Path, &Path)],
    env_var_pio_conf_prefix: Option<impl AsRef<str>>,
    quick_dump: bool,
    dump_only: bool,
) -> Result<()> {
    let path = path.as_ref();

    //let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path)?;

    create_platformio_ini(path, resolution, patches, env_var_pio_conf_prefix, quick_dump, dump_only)?;
    create_platformio_dump_py(path)?;
    create_platformio_patch_py(path)?;
    create_c_entry_points(path)?;

    Ok(())
}

fn build_project(
    pio: &Pio,
    project_path: impl AsRef<Path>,
    release: bool,
) -> Result<SconsVariables> {
    let mut cmd = pio.run_cmd();

    cmd
        .arg("-d")
        .arg(project_path.as_ref())
        .arg("-e")
        .arg(if release {"release"} else {"debug"});

    pio.exec(&mut cmd)?;

    SconsVariables::from_json(project_path)
}

fn copy_files(project_path: impl AsRef<Path>, env_var_file_copy_prefix: Option<impl AsRef<str>>) -> Result<()> {
    if let Some(env_var_file_copy_prefix) = env_var_file_copy_prefix {
        for i in 0 .. 99 {
            if let Ok(glob) = env::var(format!("{}{}", env_var_file_copy_prefix.as_ref(), i)) {
                let base = PathBuf::from(env::var(format!("{}BASE", env_var_file_copy_prefix.as_ref()))?);

                let walker = globwalk::GlobWalkerBuilder::from_patterns(&base, &[glob.as_str()])
                    .follow_links(true)
                    .build()?
                    .into_iter()
                    .filter_map(Result::ok);

                for entry in walker {
                    let file = entry.path();
                    let dest_file = project_path.as_ref().join(file.strip_prefix(&base)?);

                    fs::create_dir_all(dest_file.parent().ok_or(anyhow::format_err!("Unexpected"))?)?;
                    fs::copy(&file, dest_file)?;

                    println!("cargo:rerun-if-changed={}", file.display());
                }
            }
        }
    }

    Ok(())
}

fn apply_patches(project_path: impl AsRef<Path>, patches: &[(impl AsRef<Path>, impl AsRef<Path>)]) -> Result<()> {
    let patches_path = project_path.as_ref().join("patches");

    for patch in patches {
        let patch = patch.0.as_ref();

        fs::create_dir_all(&patches_path)?;
        fs::copy(patch, patches_path.join(patch.file_name().ok_or(anyhow::anyhow!("Invalid patch name"))?))?;
    }

    Ok(())
}

fn create_platformio_ini(
    path: impl AsRef<Path>,
    resolution: &Resolution,
    patches: &[(impl AsRef<Path>, impl AsRef<Path>)],
    env_var_pio_conf_prefix: Option<impl AsRef<str>>,
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
extra_scripts = {}platformio.dump.py
board = {}
platform = {}
framework = {}
quick_dump = {}
terminate_after_dump = {}
{}{}

[env:debug]
build_type = debug

[env:release]
build_type = release
"#,
        if patches.len() > 0 {"pre:platformio.patch.py, "} else {""},
        resolution.board,
        resolution.platform,
        resolution.frameworks.join(", "),
        quick_dump,
        dump_only,
        configure_pio_patches(patches)?,
        get_custom_pio_options(env_var_pio_conf_prefix)?,
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

fn create_platformio_patch_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.patch.py");

    fs::write(path.as_ref().join("platformio.patch.py"), PLATFORMIO_PATCH_PY)?;

    Ok(())
}

fn create_platformio_dump_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating platformio.dump.py");

    fs::write(path.as_ref().join("platformio.dump.py"), PLATFORMIO_DUMP_PY)?;

    Ok(())
}

fn configure_pio_patches(patches: &[(impl AsRef<Path>, impl AsRef<Path>)]) -> Result<String> {
    let result = patches
        .into_iter()
        .map(|pair| format!(
            "{}@{}",
            pair.1.as_ref().display(),
            pair.0.as_ref().file_name().unwrap().to_string_lossy()))
        .collect::<Vec<String>>()
        .join("\n");

    Ok(if !result.is_empty() {
        format!("patches = {}\n", result)
    } else {
        result
    })
}

fn get_custom_pio_options(env_var_pio_conf_prefix: Option<impl AsRef<str>>) -> Result<String> {
    let mut result = Vec::new();

    if let Some(env_var_pio_conf_prefix) = env_var_pio_conf_prefix {
        for i in 0 .. 99 {
            if let Ok(option) = env::var(format!("{}{}", env_var_pio_conf_prefix.as_ref(), i)) {
                result.push(option);
            }
        }
    }

    Ok(result.join("\n"))
}
