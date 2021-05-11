use std::{env, path::{Path, PathBuf}};

use anyhow::*;
use log::*;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};

use pio::*;
use pio::cargo::*;

const CMD_PIO: &'static str = "pio";
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
const PARAM_INIT_TARGET: &'static str = "target";
const PARAM_INIT_BUILD_STD: &'static str = "build-std";
const PARAM_PIO_DIR: &'static str = "pio-installation";
const PARAM_VERBOSE: &'static str = "verbose";
const PARAM_QUIET: &'static str = "quiet";

fn main() -> anyhow::Result<()> {
    let mut args = env::args();
    args.next(); // Skip over the command-line executable

    run(args.next() == Some(CMD_PIO.to_owned()))
}

fn run(as_plugin: bool) -> Result<()> {
    let mut matches = &app(as_plugin).get_matches();

    if as_plugin {
        matches = matches.subcommand_matches(CMD_PIO).unwrap();
    }

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
                    target: args.value_of(PARAM_INIT_TARGET).map(str::to_owned),
                },
                match args.value_of(PARAM_INIT_BUILD_STD) {
                    Some("std") => BuildStd::Std,
                    Some("core") => BuildStd::Core,
                    _ => BuildStd::None,
                },
                get_args(args, ARG_CARGO_ARGS)),
        _ => {
            app(as_plugin).print_help()?;
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
        build_std: BuildStd,
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
        let resolution = resolution.as_ref().unwrap();

        create_platformio_ini(
            &path,
            rust_lib,
            resolution.target.as_str(),
            resolution)?;

        create_cargo_settings(&path, build_std, Some(resolution.target.as_str()))?;

        create_entry_points(&path)?;
        create_dummy_c_file(&path)?;
        update_gitignore(&path)?;
    }

    create_cargo_py(&path)?;

    Ok(())
}

fn get_args(args: &ArgMatches, raw_arg_name: &str) -> Vec<String> {
    args
        .values_of(raw_arg_name)
        .map(|args| args.map(|s| s.to_owned()).collect::<Vec<_>>())
        .unwrap_or(Vec::new())
}

fn app<'a, 'b>(as_plugin: bool) -> App<'a, 'b> {
    let app = App::new(if as_plugin {"cargo"} else {"cargo-pio"})
        .setting(AppSettings::DeriveDisplayOrder)
        .version("0.1")
        .author("Ivan Markov")
        .about("Cargo <-> PlatformIO integration. Build Rust embedded projects with PlatformIO!");

    if as_plugin {
        app
            .bin_name("cargo")
            .subcommand(real_app(SubCommand::with_name(CMD_PIO)))
    } else {
        real_app(app)
    }
}

fn real_app<'a, 'b>(app: App<'a, 'b>) -> App<'a, 'b> {
    app
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
        Arg::with_name(PARAM_INIT_TARGET)
            .short("t")
            .long("target")
            .takes_value(true)
            .help("Rust target to be used. Defaults to a target derived from the board MCU"),
        Arg::with_name(PARAM_INIT_BUILD_STD)
            .short("s")
            .long("build-std")
            .takes_value(true)
            .help("Creates an [unstable] section in .cargo/config.toml that builds either Core, or STD. Useful for targets that do not have Core or STD pre-built. Accepted values: none, core, std"),
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
