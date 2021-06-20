use std::{env, path::PathBuf};

use anyhow::*;
use log::*;

use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};

use pio::*;

const CMD_PIO: &'static str = "pio";
const CMD_INSTALLPIO: &'static str = "installpio";
const CMD_CHECKPIO: &'static str = "checkpio";
const CMD_NEW: &'static str = "new";
const CMD_INIT: &'static str = "init";
const CMD_UPGRADE: &'static str = "upgrade";
const CMD_UPDATE: &'static str = "update";
const CMD_BUILD: &'static str = "build";
const CMD_EXECPIO: &'static str = "exec";
const CMD_PRINT_SCONS: &'static str = "printscons";

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
const PARAM_PRINT_SCONS_VAR: &'static str = "var";
const PARAM_PRINT_SCONS_PRECISE: &'static str = "precise";
const PARAM_PIO_DIR: &'static str = "pio-installation";
const PARAM_VERBOSE: &'static str = "verbose";
const PARAM_QUIET: &'static str = "quiet";

fn main() -> Result<()> {
    let mut args = env::args();
    args.next(); // Skip over the command-line executable

    run(args.next() == Some(CMD_PIO.to_owned()))
}

fn run(as_plugin: bool) -> Result<()> {
    let mut matches = &app(as_plugin).get_matches();

    if as_plugin {
        matches = matches.subcommand_matches(CMD_PIO).unwrap();
    }

    let pio_log_level = if matches.is_present(PARAM_QUIET) {
        LogLevel::Quiet
    } else if matches.is_present(PARAM_VERBOSE) {
        LogLevel::Verbose
    } else {
        LogLevel::Standard
    };

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
            Pio::install(args.value_of(ARG_PATH), pio_log_level, false)?;
            Ok(())
        },
        (CMD_CHECKPIO, Some(args)) => {
            Pio::get(args.value_of(ARG_PATH), pio_log_level, false)?;
            Ok(())
        },
        (CMD_PRINT_SCONS, Some(args)) => {
            let pio = Pio::get(args.value_of(PARAM_PIO_DIR), pio_log_level, false)?;

            let scons_vars = cargofirst::get_framework_scons_vars(
                &pio,
                args.is_present(PARAM_BUILD_RELEASE),
                !args.is_present(PARAM_PRINT_SCONS_PRECISE),
                &resolve(pio.clone(), args)?)?;

            if args.is_present(PARAM_PRINT_SCONS_VAR) {
                let scons_var = match args.value_of(PARAM_PRINT_SCONS_VAR).unwrap() {
                    "path" => scons_vars.path,
                    "incflags" => scons_vars.incflags,
                    "libflags" => scons_vars.libflags,
                    "libdirflags" => scons_vars.libdirflags,
                    "libs" => scons_vars.libs,
                    "linkflags" => scons_vars.linkflags,
                    "link" => scons_vars.link,
                    "linkcom" => scons_vars.linkcom,
                    "mcu" => scons_vars.mcu,
                    "clangargs" => scons_vars.clangargs.unwrap_or("".into()),
                    other => bail!("Unknown PlatformIO SCONS variable: {}", other),
                };

                println!("{}", scons_var);
            } else {
                println!("{:?}", scons_vars);
            }

            Ok(())
        },
        (CMD_BUILD, Some(args)) =>
            Pio::get(args.value_of(PARAM_PIO_DIR), pio_log_level, false)?
                .run_with_args(if args.is_present(PARAM_BUILD_RELEASE) {&["-e", "release"]} else {&["-e", "debug"]}),
        (CMD_EXECPIO, Some(args)) =>
            Pio::get(args.value_of(PARAM_PIO_DIR), pio_log_level, false)?
                .exec_with_args(get_args(&args, ARG_PIO_ARGS).as_slice()),
        (cmd @ CMD_NEW, Some(args))
                | (cmd @ CMD_INIT, Some(args))
                | (cmd @ CMD_UPGRADE, Some(args))
                | (cmd @ CMD_UPDATE, Some(args)) =>
            piofirst::regenerate_project(
                cmd == CMD_NEW,
                cmd == CMD_INIT || cmd == CMD_NEW,
                cmd == CMD_INIT || cmd == CMD_NEW || cmd == CMD_UPGRADE,
                args.value_of(ARG_PATH)
                    .map(PathBuf::from)
                    .unwrap_or(env::current_dir()?),
                args.value_of(ARG_PATH),
                if cmd != CMD_UPDATE {
                    Some(resolve(Pio::get(args.value_of(PARAM_PIO_DIR), pio_log_level, false/*download*/)?, args)?)
                } else {
                    None
                },
                match args.value_of(PARAM_INIT_BUILD_STD) {
                    Some("std") => piofirst::BuildStd::Std,
                    Some("core") => piofirst::BuildStd::Core,
                    _ => piofirst::BuildStd::None,
                },
                get_args(args, ARG_CARGO_ARGS),
            ),
        _ => {
            app(as_plugin).print_help()?;
            println!();

            Ok(())
        },
    }
}

fn resolve(pio: Pio, args: &ArgMatches) -> Result<Resolution> {
    Resolver::new(pio)
        .params(ResolutionParams {
            board: args.value_of(PARAM_INIT_BOARD).map(str::to_owned),
            mcu: args.value_of(PARAM_INIT_MCU).map(str::to_owned),
            platform: args.value_of(PARAM_INIT_PLATFORM).map(str::to_owned),
            frameworks: get_args(args, PARAM_INIT_FRAMEWORKS),
            target: args.value_of(PARAM_INIT_TARGET).map(str::to_owned),
        })
        .resolve()
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
        .args(&std_args())
        .subcommand(SubCommand::with_name(CMD_INSTALLPIO)
            .about("Installs PlatformIO")
            .arg(Arg::with_name(ARG_PATH)
                .required(false)
                .help("The directory where PlatformIO should be installed. Defaults to ~/.platformio")))
        .subcommand(SubCommand::with_name(CMD_CHECKPIO)
            .about("Checks whether PlatformIO is installed")
            .arg(Arg::with_name(ARG_PATH)
                .required(false)
                .help("PlatformIO installation directory to be checked. Defaults to ~/.platformio")))
        .subcommand(SubCommand::with_name(CMD_PRINT_SCONS)
            .about("Prints one or all Scons environment variables that would be used when PlatformIO builds a project")
            .args(&platformio_framework_args())
            .arg(Arg::with_name(PARAM_PRINT_SCONS_PRECISE)
                .long("precise")
                .help("Precise Scons environment variables calculation. Simulates a real PlatformIO build")
                .required(false))
            .arg(Arg::with_name(PARAM_PRINT_SCONS_VAR)
                .short("s")
                .long("var")
                .required(false)
                .takes_value(true)
                .possible_values(&["path", "incflags", "libflags", "libdirflags", "libs", "linkflags", "link", "linkcom", "mcu", "clangargs"])
                .help("PlatformIO Scons environment variable to print.")))
        .subcommand(SubCommand::with_name(CMD_NEW)
            .about("Creates a new PIO->Cargo project")
            .args(&platformio_ini_args())
            .args(&init_args(true)))
        .subcommand(SubCommand::with_name(CMD_INIT)
            .about("Creates a new PIO->Cargo project in an existing directory")
            .args(&platformio_ini_args())
            .args(&init_args(false)))
        .subcommand(SubCommand::with_name(CMD_UPGRADE)
            .about("Upgrades an existing Cargo library crate to a PIO->Cargo project")
            .args(&platformio_ini_args())
            .arg(Arg::with_name(ARG_PATH)
                .help("The directory of the existing Cargo library crate. Defaults to the current directory")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_UPDATE)
            .about("Updates an existing PIO->Cargo project with the latest PlatformIO=>Cargo integration scripts")
            .arg(pio_installation_arg())
            .arg(Arg::with_name(ARG_PATH)
                .help("The directory of the existing PIO->Cargo project. Defaults to the current directory")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_BUILD)
            .about("Builds a PIO->Cargo project (both the Cargo library crate and the PlatformIO build).\nEquivalent to executing subcommand 'exec -- run -e debug'")
            .arg(pio_installation_arg())
            .arg(Arg::with_name(PARAM_BUILD_RELEASE)
                .short("r")
                .long("release")
                .help("Perform a release build. Equivalent to executing subcommand 'exec -- run -e release'")
                .required(false)))
        .subcommand(SubCommand::with_name(CMD_EXECPIO)
            .about("Executes PlatformIO in the current directory")
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

fn platformio_framework_args<'a, 'b>() -> Vec<Arg<'a, 'b>> {
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
    ]
}

fn platformio_ini_args<'a, 'b>() -> Vec<Arg<'a, 'b>> {
    let mut args = platformio_framework_args();
    args.push(Arg::with_name(PARAM_INIT_BUILD_STD)
        .short("s")
        .long("build-std")
        .takes_value(true)
        .help("Creates an [unstable] section in .cargo/config.toml that builds either Core, or STD. Useful for targets that do not have Core or STD pre-built. Accepted values: none, core, std"));

    args
}

fn init_args<'a, 'b>(path_required: bool) -> Vec<Arg<'a, 'b>> {
    vec![
        Arg::with_name(ARG_PATH)
            .help(if !path_required {
                    "The directory where the PIO->Cargo project should be created. Defaults to the current directory"
                } else {
                    "The directory where the PIO->Cargo project should be created"
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
        .help("PlatformIO installation directory (default is ~/.platformio)")
}
