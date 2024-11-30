use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{bail, Result};
use embuild::cargo::CargoCmd;
use embuild::pio::*;
use embuild::*;
use log::*;
use structopt::StructOpt;
use tempfile::TempDir;

const PLATFORMIO_ESP32_EXCEPTION_DECODER_DIFF: &[u8] =
    include_bytes!("patches/filter_exception_decoder_esp32c3_external_conf_fix.diff");

#[derive(Debug, StructOpt)]
#[structopt(
    author,
    about = "Cargo <-> PlatformIO integration. Build Rust embedded projects with PlatformIO!",
    setting = structopt::clap::AppSettings::DeriveDisplayOrder
)]
struct Opt {
    /// Prints verbose output
    #[structopt(short, long)]
    verbose: bool,
    /// Stay quiet, don't print any output
    #[structopt(short, long)]
    quiet: bool,

    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(Debug, StructOpt)]
enum Command {
    /// Installs PlatformIO
    Installpio {
        /// The directory where PlatformIO should be installed. Defaults to ~/.platformio
        #[structopt(parse(from_os_str))]
        path: Option<PathBuf>,
    },
    /// Checks whether PlatformIO is installed
    Checkpio {
        /// PlatformIO installation directory to be checked. Defaults to ~/.platformio
        #[structopt(parse(from_os_str))]
        path: Option<PathBuf>,
    },
    /// Prints one or all Scons environment variables that would be used when PlatformIO builds a project
    Printscons {
        #[structopt(flatten)]
        framework_args: PioFrameworkArgs,

        /// Precise Scons environment variables calculation. Simulates a real PlatformIO build
        #[structopt(long)]
        precise: bool,

        /// Do a release build
        #[structopt(short, long)]
        release: bool,

        /// PlatformIO Scons environment variable to print
        #[structopt(short = "s", long,
                    possible_values = &["path", "incflags", "libflags", "libdirflags", "libs",
                                        "linkflags", "link", "linkcom", "mcu", "clangargs"])]
        var: Option<String>,
    },
    /// Creates a new PIO->Cargo project
    New {
        #[structopt(flatten)]
        pio_ini_args: PioIniArgs,

        /// The directory where the PIO->Cargo project should be created
        #[structopt(parse(from_os_str))]
        path: PathBuf,

        /// Pass-through arguments down to Cargo
        #[structopt(required = false, allow_hyphen_values = true, last = true)]
        cargo_args: Vec<String>,
    },
    /// Creates a new PIO->Cargo project in an existing directory
    Init {
        #[structopt(flatten)]
        pio_ini_args: PioIniArgs,

        /// The directory where the PIO->Cargo project should be created
        #[structopt(parse(from_os_str))]
        path: Option<PathBuf>,

        /// Pass-through arguments down to Cargo
        #[structopt(required = false, allow_hyphen_values = true, last = true)]
        cargo_args: Vec<String>,
    },
    /// Upgrades an existing Cargo library crate to a PIO->Cargo project
    Upgrade {
        #[structopt(flatten)]
        pio_ini_args: PioIniArgs,

        /// The directory of the existing Cargo library crate. Defaults to the current directory
        #[structopt(parse(from_os_str))]
        path: Option<PathBuf>,

        /// Pass-through arguments down to Cargo
        #[structopt(required = false, allow_hyphen_values = true, last = true)]
        cargo_args: Vec<String>,
    },
    /// Updates an existing PIO->Cargo project with the latest PlatformIO=>Cargo integration scripts
    Update {
        /// The directory of the existing Cargo library crate. Defaults to the current directory
        #[structopt(parse(from_os_str))]
        path: Option<PathBuf>,
    },
    /// Builds a PIO->Cargo project (both the Cargo library crate and the PlatformIO build)
    ///
    /// Equivalent to executing subcommand 'exec -- run -e debug'
    Build {
        #[structopt(flatten)]
        pio_install: PioInstallation,

        /// Performs a release build
        ///
        /// Equivalent to executing subcommand 'exec -- run -e release'
        #[structopt(long, short)]
        release: bool,
    },
    /// Executes PlatformIO in the current directory
    Exec {
        #[structopt(flatten)]
        pio_install: PioInstallation,

        /// Pass-through arguments down to PlatformIO
        #[structopt(required = false, allow_hyphen_values = true, last = true)]
        pio_args: Vec<OsString>,
    },
    /// Invokes commands specific for the ESP-IDF SDK
    Espidf {
        #[structopt(flatten)]
        pio_install: PioInstallation,

        #[structopt(subcommand)]
        cmd: EspidfCommand,
    },
}

#[derive(Debug, StructOpt)]
struct PioInstallation {
    /// PlatformIO installation directory (default is ~/.platformio)
    #[structopt(short = "i", long = "pio-installation")]
    pio_path: Option<PathBuf>,
}

#[derive(Debug, StructOpt)]
struct PioFrameworkArgs {
    #[structopt(flatten)]
    pio_install: PioInstallation,

    /// Resolves the PlatformIO project with this board ID
    #[structopt(short, long)]
    board: Option<String>,

    /// Resolves the PlatformIO project with this MCU ID
    #[structopt(short, long)]
    mcu: Option<String>,

    /// Resolves the PlatformIO project with this platform ID
    #[structopt(short, long)]
    platform: Option<String>,

    /// Resolves the PlatformIO project with this framework ID(s)
    #[structopt(short, long)]
    frameworks: Option<Vec<String>>,

    /// Rust target to be used. Defaults to a target derived from the board MCU
    #[structopt(short, long)]
    target: Option<String>,
}

#[derive(Debug, StructOpt)]
struct PioIniArgs {
    #[structopt(flatten)]
    framework_args: PioFrameworkArgs,
    /// Selects which part of the standard library cargo should build
    ///
    /// If not set to `none` create an `[unstable]` section in `.cargo/config.toml` that
    /// specifies if `core` or `std` should be built by cargo. Useful for targets that do
    /// not have `core` or `std` pre-built.
    #[structopt(short = "s", long, parse(from_str = parse_build_std),
                default_value = "core", possible_values = &["none", "core", "std"])]
    build_std: cargo::BuildStd,
}

#[derive(Debug, StructOpt)]
enum EspidfCommand {
    /// Generates or updates the ESP-IDF sdkconfig file using the ESP-IDF Menuconfig interactive system
    Menuconfig {
        /// Rust target for which the sdkconfig file will be generated or updated
        #[structopt(short, long)]
        target: Option<String>,

        /// Indicates release configuration
        ///
        /// Equivalent to '-e release'
        #[structopt(long, short)]
        release: Option<bool>,

        /// PlatformIO environment to configure
        ///
        /// If not specified, the PlatformIO project default environment will be used (or error will be generated if there isn't one)
        #[structopt(long, short = "e")]
        environment: Option<String>,
    },
    /// Invokes the PlatformIO monitor
    Monitor {
        /// Port
        #[structopt()]
        port: String,

        /// Baud rate. Defaults to 115200
        #[structopt(short = "b", long)]
        baud_rate: Option<u32>,

        /// Do not apply encodings/transformations
        #[structopt(long)]
        raw: bool,

        /// Binary name built by this crate for which the monitor will be invoked (necessary for access to the ELF file)
        #[structopt(long)]
        binary: Option<String>,

        /// Rust target for which the monitor will be invoked (necessary for access to the ELF file)
        #[structopt(short, long)]
        target: Option<String>,

        /// Indicates release configuration
        ///
        /// Equivalent to '-e release'
        #[structopt(long, short)]
        release: Option<bool>,

        /// PlatformIO environment to monitor
        ///
        /// If not specified, the PlatformIO project default environment will be used (or error will be generated if there isn't one)
        #[structopt(long, short = "e")]
        environment: Option<String>,
    },
}

fn parse_build_std(s: &str) -> cargo::BuildStd {
    match s {
        "none" => cargo::BuildStd::None,
        "core" => cargo::BuildStd::Core,
        "std" => cargo::BuildStd::Std,
        _ => panic!(),
    }
}

impl PioFrameworkArgs {
    fn resolve(self, pio: Pio) -> Result<Resolution> {
        Resolver::new(pio)
            .params(ResolutionParams {
                board: self.board,
                mcu: self.mcu,
                platform: self.platform,
                frameworks: self.frameworks.unwrap_or_default(),
                target: self.target,
            })
            .resolve(false)
    }
}

fn main() -> Result<()> {
    let as_plugin = env::args().nth(1).iter().any(|s| s == "pio");

    let args = env::args_os().skip(as_plugin as usize);
    let opt = Opt::from_iter(args);

    let pio_log_level = if opt.quiet {
        LogLevel::Quiet
    } else if opt.verbose {
        LogLevel::Verbose
    } else {
        LogLevel::Standard
    };

    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("CARGO_PIO_LOG_STYLE", "Auto")
            .filter_or(
                "CARGO_PIO_LOG",
                (if opt.quiet {
                    LevelFilter::Warn
                } else if opt.verbose {
                    LevelFilter::Debug
                } else {
                    LevelFilter::Info
                })
                .to_string(),
            ),
    )
    .target(env_logger::Target::Stderr)
    .format_level(false)
    .format_indent(None)
    .format_module_path(false)
    .format_timestamp(None)
    .init();

    match opt.cmd {
        Command::Installpio { path } => {
            Pio::install(path, pio_log_level, false)?;
            Ok(())
        }
        Command::Checkpio { path } => {
            Pio::get(path, pio_log_level, false)?;
            Ok(())
        }
        Command::Printscons {
            mut framework_args,
            precise,
            var,
            release,
        } => {
            let pio = Pio::get(
                framework_args.pio_install.pio_path.take(),
                pio_log_level,
                false,
            )?;

            let scons_vars = get_framework_scons_vars(
                &pio,
                release,
                !precise,
                &framework_args.resolve(pio.clone())?,
            )?;

            if let Some(var) = var {
                let scons_var = match &var[..] {
                    "path" => scons_vars.path,
                    "incflags" => scons_vars.incflags,
                    "libflags" => scons_vars.libflags,
                    "libdirflags" => scons_vars.libdirflags,
                    "libs" => scons_vars.libs,
                    "linkflags" => scons_vars.linkflags,
                    "link" => scons_vars.link,
                    "linkcom" => scons_vars.linkcom,
                    "mcu" => format!("{:?}", scons_vars.mcu),
                    "clangargs" => scons_vars.clangargs.unwrap_or_else(|| "".into()),
                    _ => panic!(),
                };

                println!("{}", scons_var);
            } else {
                println!("{:?}", scons_vars);
            }

            Ok(())
        }
        Command::Build {
            pio_install,
            release,
        } => Pio::get(pio_install.pio_path, pio_log_level, false)?.run_with_args(if release {
            &["-e", "release"]
        } else {
            &["-e", "debug"]
        }),
        Command::Exec {
            pio_install,
            pio_args: args,
        } => Pio::get(pio_install.pio_path, pio_log_level, false)?.exec_with_args(&args),
        cmd @ Command::New { .. } | cmd @ Command::Init { .. } | cmd @ Command::Upgrade { .. } => {
            let (cargo_cmd, mut pio_ini_args, path, args) = match cmd {
                Command::New {
                    pio_ini_args,
                    path,
                    cargo_args: args,
                } => (
                    CargoCmd::New(pio_ini_args.build_std),
                    pio_ini_args,
                    Some(path),
                    args,
                ),
                Command::Init {
                    pio_ini_args,
                    path,
                    cargo_args: args,
                } => (
                    CargoCmd::Init(pio_ini_args.build_std),
                    pio_ini_args,
                    path,
                    args,
                ),
                Command::Upgrade {
                    pio_ini_args,
                    path,
                    cargo_args: args,
                } => (CargoCmd::Upgrade, pio_ini_args, path, args),
                _ => unreachable!(),
            };

            let pio_path = pio_ini_args.framework_args.pio_install.pio_path.take();
            create_project(
                path.unwrap_or(env::current_dir()?),
                cargo_cmd,
                args.iter(),
                &pio_ini_args.framework_args.resolve(Pio::get(
                    pio_path,
                    pio_log_level,
                    false, /*download*/
                )?)?,
            )?;

            Ok(())
        }
        Command::Update { path } => {
            update_project(path.unwrap_or(env::current_dir()?))?;
            Ok(())
        }
        Command::Espidf {
            pio_install,
            cmd:
                EspidfCommand::Menuconfig {
                    target,
                    release,
                    environment,
                },
        } => {
            run_esp_idf_menuconfig(
                Pio::get(pio_install.pio_path, pio_log_level, false /*download*/)?,
                env::current_dir()?,
                target.as_deref(),
                if environment.is_some() {
                    environment.as_deref()
                } else if let Some(true) = release {
                    Some("release")
                } else {
                    None
                },
            )
        }
        Command::Espidf {
            pio_install,
            cmd:
                EspidfCommand::Monitor {
                    port,
                    baud_rate,
                    raw,
                    binary,
                    target,
                    release,
                    environment,
                },
        } => {
            run_esp_idf_monitor(
                Pio::get(pio_install.pio_path, pio_log_level, false /*download*/)?,
                env::current_dir()?,
                &port,
                baud_rate.unwrap_or(115200),
                raw,
                binary.as_deref(),
                target.as_deref(),
                if environment.is_some() {
                    environment.as_deref()
                } else if let Some(true) = release {
                    Some("release")
                } else {
                    None
                },
            )
        }
    }
}

fn run_esp_idf_menuconfig<'a>(
    pio: Pio,
    project: impl AsRef<Path>,
    target: Option<&'a str>,
    environment: Option<&'a str>,
) -> Result<()> {
    let args = if let Some(environment) = environment {
        vec!["-t", "menuconfig", "-e", environment]
    } else {
        vec!["-t", "menuconfig"]
    };

    if check_pio_first_project(&project) {
        call_in_dir(project, move || pio.run_with_args(&args))
    } else {
        let target = derive_target(project, target)?;

        let resolution = resolve_esp_idf_target(pio.clone(), target)?;

        let sdkconfigs = &[
            env::current_dir()?.join("sdkconfig"),
            env::current_dir()?.join("sdkconfig.debug"),
        ];

        for sdkconfig in sdkconfigs {
            if sdkconfig.exists() && sdkconfig.is_dir() {
                bail!(
                    "The sdkconfig entry {} is a directory, not a file",
                    sdkconfig.display()
                );
            }
        }

        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().join("proj");

        project::Builder::new(&project_path)
            .enable_c_entry_points()
            .generate(&resolution)?;

        for sdkconfig in sdkconfigs {
            if sdkconfig.exists() {
                let dest_sdkconfig = project_path.join(sdkconfig.file_name().unwrap());

                fs::copy(sdkconfig, dest_sdkconfig)?;
            }
        }

        call_in_dir(&project_path, move || pio.run_with_args(&args))?;

        for sdkconfig in sdkconfigs {
            let dest_sdkconfig = project_path.join(sdkconfig.file_name().unwrap());

            if dest_sdkconfig.exists() {
                fs::copy(dest_sdkconfig, sdkconfig)?;
            }
        }

        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_esp_idf_monitor<'a>(
    mut pio: Pio,
    project: impl AsRef<Path>,
    port: &'a str,
    baud_rate: u32,
    raw: bool,
    binary: Option<&'a str>,
    target: Option<&'a str>,
    environment: Option<&'a str>,
) -> Result<()> {
    let baud_rate = baud_rate.to_string();

    let mut args = vec![
        "device",
        "monitor",
        "-p",
        port,
        "-b",
        &baud_rate,
        "--filter",
        "esp32_exception_decoder",
    ];

    if raw {
        args.push("--raw");
    }

    if let Some(environment) = environment {
        args.extend(["-e", environment]);
    }

    if check_pio_first_project(&project) {
        call_in_dir(project, move || pio.exec_with_args(&args))
    } else {
        let target = derive_target(&project, target)?;

        let resolution = resolve_esp_idf_target(pio.clone(), &target)?;

        let elf_file = cargo::Crate::new(&project).get_binary_path(
            Some("release") == environment,
            Some(target.as_str()),
            binary,
        )?;
        if !elf_file.exists() {
            bail!(
                "Elf file {} does not exist, did you build your project first?",
                elf_file.display()
            );
        } else if elf_file.is_dir() {
            bail!("Elf file {} points to a directory", elf_file.display());
        }

        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().join("proj");

        project::Builder::new(&project_path)
            .enable_c_entry_points()
            .platform_package_patch(
                PathBuf::from("patches")
                    .join("filter_exception_decoder_esp32c3_external_conf_fix.diff"),
                PathBuf::from("__platform__"),
            )
            .enable_scons_dump() // Just a trick to do an early termination of the build
            .option(project::OPTION_TERMINATE_AFTER_DUMP, "true")
            .option(project::OPTION_QUICK_DUMP, "true")
            .generate(&resolution)?;

        let patch_dir = project_path.join("patches");

        fs::create_dir_all(&patch_dir)?;
        fs::write(
            patch_dir.join("filter_exception_decoder_esp32c3_external_conf_fix.diff"),
            PLATFORMIO_ESP32_EXCEPTION_DECODER_DIFF,
        )?;

        // For now, we need to build the project, as ther build is patching the esp32_exception_decoder filter
        // so that it supports the environment variables from below, and does proper stacktrace decoding for ESP32C3
        pio = pio.log_level(LogLevel::Quiet);
        pio.build(&project_path, Some("release") == environment)?;

        // Need to re-generate the project again or else the filter fails with:
        // Esp32ExceptionDecoder: disabling, exception while looking for addr2line: Warning! Ignore unknown configuration option `patches` in section [env]
        // TODO: Address this issue to PlatformIO. Euither custom configurations are supported, or not
        project::Builder::new(&project_path)
            .enable_c_entry_points()
            .generate(&resolution)?;

        let mut cmd = pio.cmd();

        cmd.env(
            "esp32_exception_decoder_project_strip_dir",
            project.as_ref().as_os_str(),
        )
        .env(
            "esp32_exception_decoder_firmware_path",
            elf_file.as_os_str(),
        )
        .args(args);

        pio = pio.log_level(LogLevel::Standard);
        call_in_dir(project_path, move || pio.exec(&mut cmd))
    }
}

fn resolve_esp_idf_target(pio: Pio, target: impl AsRef<str>) -> Result<Resolution> {
    Resolver::new(pio)
        .params(ResolutionParams {
            platform: Some("espressif32".into()),
            frameworks: vec!["espidf".into()],
            target: Some(target.as_ref().to_owned()),
            ..Default::default()
        })
        .resolve(true)
}

fn check_pio_first_project(project: impl AsRef<Path>) -> bool {
    let project = project.as_ref();

    let platformio_ini = project.join("platformio.ini");

    if platformio_ini.exists() && platformio_ini.is_file() {
        // We are running the monitor on a Pio-first project (possibly a PIO->Cargo one)
        // Just run the PlatformIO monitor then
        info!("Found platformio.ini in {}", project.display());

        true
    } else {
        info!(
            "platformio.ini not found in {}, assuming a Cargo-first project",
            project.display()
        );

        false
    }
}

fn call_in_dir<F, R>(dir: impl AsRef<Path>, f: F) -> Result<R>
where
    F: FnOnce() -> Result<R>,
{
    let current_dir = env::current_dir()?;

    env::set_current_dir(&dir)?;

    let result = f();

    env::set_current_dir(current_dir)?;

    result
}

fn derive_target(project: impl AsRef<Path>, target: Option<&str>) -> Result<String> {
    Ok(if let Some(target) = target {
        info!("Using explicitly passed target {}", target);

        target.to_owned()
    } else if let Some(target) = cargo::Crate::new(project).get_default_target()? {
        info!("Using pre-configured target {}", target);

        target
    } else {
        bail!("Cannot find 'target=' specification in any Cargo configuration file. Please use the --target parameter to specify the target on the command line");
    })
}

fn get_framework_scons_vars(
    pio: &Pio,
    release: bool,
    quick: bool,
    resolution: &Resolution,
) -> Result<project::SconsVariables> {
    let temp_dir = TempDir::new()?;
    let project_path = temp_dir.path().join("proj");

    let mut builder = project::Builder::new(&project_path);

    builder
        .enable_scons_dump()
        .option(project::OPTION_TERMINATE_AFTER_DUMP, "true");

    if quick {
        builder.option(project::OPTION_QUICK_DUMP, "true");
    }

    builder.generate(resolution)?;

    pio.build(&project_path, release)?;

    project::SconsVariables::from_dump(project_path)
}

fn create_project<I, S>(
    project_path: impl AsRef<Path>,
    cargo_cmd: CargoCmd,
    cargo_args: I,
    resolution: &Resolution,
) -> Result<PathBuf>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    let mut builder = project::Builder::new(project_path.as_ref());

    builder
        .enable_git_repos()
        .enable_platform_packages_patches()
        .enable_cargo(cargo_cmd)
        .cargo_options(cargo_args)
        .generate(resolution)
}

fn update_project(project_path: impl AsRef<Path>) -> Result<PathBuf> {
    project::Builder::new(project_path).update()
}
