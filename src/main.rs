use std::{env, ffi::OsStr, fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}, process::Command};

use anyhow::*;
use log::*;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use cargo_toml::{Manifest, Product};
use toml;

use pio::*;

const CARGO_PY: &'static [u8] = include_bytes!("Cargo.py");

const CMD_INSTALLPIO: &'static str = "installpio";
const CMD_CHECKPIO: &'static str = "checkpio";
const CMD_NEW: &'static str = "new";
const CMD_INIT: &'static str = "init";
const CMD_UPGRADE: &'static str = "upgrade";
const CMD_UPDATE: &'static str = "update";
const CMD_BUILD: &'static str = "build";
const CMD_RUNPIO: &'static str = "run";

const ARG_PATH: &'static str = "PATH";
const ARG_CARGO_ARGS: &'static str = "CARGO_ARGS";
const ARG_PIO_ARGS: &'static str = "PIO_ARGS";
const PARAM_BUILD_RELEASE: &'static str = "release";
const PARAM_INIT_BOARD: &'static str = "board";
const PARAM_INIT_MCU: &'static str = "mcu";
const PARAM_INIT_PLATFORM: &'static str = "platform";
const PARAM_INIT_FRAMEWORKS: &'static str = "frameworks";
const PARAM_PIO_DIR: &'static str = "pio-installation";
const PARAM_VERBOSE: &'static str = "verbose";
const PARAM_QUIET: &'static str = "quiet";

fn main() -> Result<()> {
    run()
}

fn run() -> Result<()> {
    let matches = app().get_matches();

    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("CARGO_PIO_LOG_STYLE", "Auto")
            .filter_or(
                "CARGO_PIO_LOG",
                (
                        if matches.is_present(PARAM_QUIET) {LevelFilter::Warn}
                        else if matches.is_present(PARAM_VERBOSE) {LevelFilter::Debug} else {LevelFilter::Info})
                    .to_string()))
        .target(env_logger::Target::Stderr)
        .format_level(false)
        .format_indent(None)
        .format_module_path(false)
        .format_timestamp(None)
        .init();

    match matches.subcommand() {
        (CMD_INSTALLPIO, Some(args)) => {
            install_platformio(args.value_of(ARG_PATH), false)?;
            Ok(())
        },
        (CMD_CHECKPIO, Some(args)) => {
            get_platformio(args.value_of(ARG_PATH), false)?;
            Ok(())
        },
        (CMD_BUILD, Some(args)) =>
            run_platformio(
                get_platformio(args.value_of(PARAM_PIO_DIR), false)?,
                if args.is_present(PARAM_BUILD_RELEASE) {&["-e", "release"]} else {&["-e", "debug"]}),
        (CMD_RUNPIO, Some(args)) =>
            run_platformio(
                get_platformio(args.value_of(PARAM_PIO_DIR), false)?,
                get_args(&args, ARG_PIO_ARGS).as_slice()),
        (cmd @ CMD_NEW, Some(args))
                | (cmd @ CMD_INIT, Some(args))
                | (cmd @ CMD_UPGRADE, Some(args))
                | (cmd @ CMD_UPDATE, Some(args)) =>
            update_project(
                cmd,
                args.value_of(PARAM_PIO_DIR),
                args.value_of(ARG_PATH)
                    .map(PathBuf::from)
                    .unwrap_or(env::current_dir()?),
                args.value_of(ARG_PATH),
                ResolutionParams {
                    board: args.value_of(PARAM_INIT_BOARD).map(str::to_owned),
                    mcu: args.value_of(PARAM_INIT_MCU).map(str::to_owned),
                    platform: args.value_of(PARAM_INIT_PLATFORM).map(str::to_owned),
                    frameworks: get_args(args, PARAM_INIT_FRAMEWORKS),
                },
                get_args(args, ARG_CARGO_ARGS)),
        _ => {
            app().print_help()?;
            println!();

            Ok(())
        },
    }
}

fn update_project(
        cmd: &str,
        pio_dir: Option<&str>,
        path: impl AsRef<Path>,
        path_opt: Option<impl AsRef<Path>>,
        resolution_params: ResolutionParams,
        cargo_args: Vec<String>) -> Result<()> {
    let create = cmd == CMD_INIT || cmd == CMD_NEW;

    let resolution = if create {
        debug!("Resolving {:?}", resolution_params);

        Some(resolve_platformio_ini(
            get_platformio(pio_dir, false)?,
            resolution_params)?)
    } else {
        None
    };

    let rust_lib = if create {
        generate_crate(cmd == CMD_NEW, &path, path_opt, cargo_args)?
    } else {
        check_crate(&path)?
    };

    if create {
        create_platformio_ini(
            &path,
            rust_lib,
            resolution.unwrap())?;

        create_cargo_settings(&path)?;
        create_dummy_c_file(&path)?;
        update_gitignore(&path)?;
    }

    create_cargo_py(&path)?;

    Ok(())
}

fn generate_crate(
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

    let name_from_dir = cargo_toml_path
        .parent().unwrap()
        .file_name().unwrap()
        .to_str().unwrap()
        .to_owned();

    let name = cargo_toml.lib.as_ref()
        .map(|lib| lib.name.clone())
        .flatten()
        .unwrap_or(cargo_toml.package.as_ref()
            .map(|package| package.name.clone())
            .unwrap_or(name_from_dir));

    debug!("Setting Cargo library crate to type \"staticlib\"");

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

    let cargo_toml = Manifest::from_path(cargo_toml_path)?;

    if let Some(lib) = cargo_toml.lib {
        let crate_type = lib.crate_type.unwrap_or(Vec::new());

        if crate_type.into_iter().find(|s| s == "staticlib").is_some() {
            Ok(lib.name.unwrap())
        } else {
            bail!("This library crate is missing a crate_type = [\"staticlib\"] declaration");
        }
    } else {
        bail!("Not a library crate");
    }
}

fn resolve_platformio_ini(pio: Pio, params: ResolutionParams) -> Result<Resolution> {
    Resolver::new(pio).params(params).resolve()
}

fn create_platformio_ini(
        path: impl AsRef<Path>,
        rust_lib: impl AsRef<str>,
        resolution: Resolution) -> Result<()> {
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
board = {}
platform = {}
framework = {}

[env:debug]
build_type = debug

[env:release]
build_type = release
"#,
        rust_lib.as_ref(),
        resolution.board,
        resolution.platform,
        resolution.frameworks.join(", ")).as_bytes())?;

    Ok(())
}

fn create_cargo_settings(path: impl AsRef<Path>) -> Result<()> {
    let cargo_config_toml_path = path.as_ref().join(".cargo").join("config.toml");

    debug!("Creating a Cargo config {} so that STD is built too", cargo_config_toml_path.display());

    fs::create_dir_all(cargo_config_toml_path.parent().unwrap())?;
    fs::write(cargo_config_toml_path, r#"
[unstable]
# If your toolchain does not support STD, change "std" below to "core" and put #![no_std] in src/lib.rs
build-std = ["std", "panic_abort"]
build-std-features = ["panic_immediate_abort"]
"#)?;

    Ok(())
}

fn create_dummy_c_file(path: impl AsRef<Path>) -> Result<()> {
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

fn update_gitignore(path: impl AsRef<Path>) -> Result<()> {
    debug!("Adding \".pio\" and \"CMakeFiles\" directories to .gitignore");

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(path.as_ref().join(".gitignore"))?;

    writeln!(file, ".pio\nCMakeFiles/")?;

    Ok(())
}

fn create_cargo_py(path: impl AsRef<Path>) -> Result<()> {
    debug!("Creating/updating Cargo.py");

    fs::write(path.as_ref().join("Cargo.py"), CARGO_PY)?;

    Ok(())
}

fn run_platformio<'a, 'b>(pio: Pio, args: &[impl AsRef<OsStr>]) -> Result<()> {
    let mut cmd = pio.cmd();

    cmd
        .arg("run")
        .args(args);

    debug!("Running PlatformIO: {:?}", cmd);

    cmd.status()?;

    Ok(())
}

fn install_platformio(pio_dir: Option<impl AsRef<Path>>, download: bool) -> Result<Pio> {
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

fn get_platformio(pio_dir: Option<impl AsRef<Path>>, download: bool) -> Result<Pio> {
    let mut pio_installer = if download { PioInstaller::new_download()? } else { PioInstaller::new()? };

    if let Some(pio_dir) = pio_dir {
        pio_installer.pio(pio_dir.as_ref());
    }

    pio_installer.check()
}

fn get_args(args: &ArgMatches, raw_arg_name: &str) -> Vec<String> {
    args
        .values_of(raw_arg_name)
        .map(|args| args.map(|s| s.to_owned()).collect::<Vec<_>>())
        .unwrap_or(Vec::new())
}

fn app<'a, 'b>() -> App<'a, 'b> {
    App::new("cargo-pio")
        .setting(AppSettings::DeriveDisplayOrder)
        .version("0.1")
        .author("Ivan Markov")
        .about("Cargo <-> PlatformIO integration. Build Rust embedded projects with PlatformIO!")
        .subcommand(SubCommand::with_name(CMD_INSTALLPIO)
            .about("Installs PlatformIO")
            .args(&std_args())
            .arg(Arg::with_name(ARG_PATH)
                .required(false)
                .help("The directory where PlatformIO should be installed. Defaults to ~/.pio")))
        .subcommand(SubCommand::with_name(CMD_CHECKPIO)
            .about("Checks whether PlatformIO is installed")
            .args(&std_args())
            .arg(Arg::with_name(ARG_PATH)
                .required(false)
                .help("PlatformIO installation directory to be checked. Defaults to ~/.pio")))
        .subcommand(SubCommand::with_name(CMD_NEW)
            .about("Creates a new Cargo/PIO project")
            .args(&std_args())
            .args(&platformio_ini_args())
            .args(&init_args(true)))
        .subcommand(SubCommand::with_name(CMD_INIT)
            .about("Creates a new Cargo/PIO project in an existing directory")
            .args(&std_args())
            .args(&platformio_ini_args())
            .args(&init_args(false)))
        .subcommand(SubCommand::with_name(CMD_UPGRADE)
            .about("Upgrades an existing Cargo library crate to a Cargo/PIO project")
            .args(&std_args())
            .args(&platformio_ini_args())
            .arg(Arg::with_name(ARG_PATH)
                .help("The directory of the existing Cargo library crate. Defaults to the current directory")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_UPDATE)
            .about("Updates an existing Cargo/PIO project to the latest Cargo.py integration script")
            .args(&std_args())
            .arg(pio_installation_arg())
            .arg(Arg::with_name(ARG_PATH)
                .help("The directory of the existing Cargo/PIO project. Defaults to the current directory")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_BUILD)
            .about("Builds the Cargo/IO project (both the Cargo library crate and the PlatformIO build).\nEquivalent to executing subcommand 'run -e debug'")
            .args(&std_args())
            .arg(pio_installation_arg())
            .arg(Arg::with_name(PARAM_BUILD_RELEASE)
                .short("r")
                .long("release")
                .help("Perform a release build. Equivalent to executing subcommand 'run -e release'")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_RUNPIO)
            .about("Executes PlatformIO 'run' in the current directory")
            .args(&std_args())
            .arg(pio_installation_arg())
            .arg(Arg::with_name(ARG_PIO_ARGS)
                .help("Pass-through arguments down to PlatformIO")
                .required(false)
                .multiple(true)
                .allow_hyphen_values(true)
                .last(true)))
}

fn std_args<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![
        Arg::with_name(PARAM_VERBOSE)
            .short("v")
            .long("verbose")
            .help("Uses verbose output"),
        Arg::with_name(PARAM_QUIET)
            .short("q")
            .long("quiet")
            .help("Stays quiet")
    ]
}

fn platformio_ini_args<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    vec![
        pio_installation_arg(),
        Arg::with_name(PARAM_INIT_BOARD)
            .short("b")
            .long("board")
            .takes_value(true)
            .help("Resolves the PlatformIO project with this board ID"),
        Arg::with_name(PARAM_INIT_MCU)
            .short("m")
            .long("mcu")
            .takes_value(true)
            .help("Resolves the PlatformIO project with this MCU ID"),
        Arg::with_name(PARAM_INIT_PLATFORM)
            .short("p")
            .long("platform")
            .takes_value(true)
            .help("Resolves the PlatformIO project with this platform ID"),
        Arg::with_name(PARAM_INIT_FRAMEWORKS)
            .short("f")
            .long("frameworks")
            .takes_value(true)
            .multiple(true)
            .help("Resolves the PlatformIO project with this framework ID(s)"),
    ]
}

fn init_args<'a, 'b>(path_required: bool) -> Vec<Arg<'a, 'b>> {
    vec![
        Arg::with_name(ARG_PATH)
            .help(if !path_required {
                    "The directory where the Cargo/PIO project should be created. Defaults to the current directory"
                } else {
                    "The directory where the Cargo/PIO project should be created"
                })
            .required(path_required),
        Arg::with_name(ARG_CARGO_ARGS)
            .help("Pass-through arguments down to Cargo")
            .required(false)
            .multiple(true)
            .allow_hyphen_values(true)
            .last(true)
    ]
}

fn pio_installation_arg<'a, 'b>() -> Arg<'a, 'b> {
    Arg::with_name(PARAM_PIO_DIR)
        .short("i")
        .long("pio-installation")
        .help("PlatformIO installation directory (default is ~/.pio)")
}
