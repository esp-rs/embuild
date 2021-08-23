use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::*;
use embuild::cargo::CargoCmd;
use embuild::pio::*;
use embuild::*;
use log::*;
use structopt::StructOpt;
use tempfile::TempDir;

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
                    "mcu" => scons_vars.mcu,
                    "clangargs" => scons_vars.clangargs.unwrap_or("".into()),
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
            cmd: EspidfCommand::Menuconfig { target },
        } => {
            run_esp_idf_menuconfig(
                Pio::get(pio_install.pio_path, pio_log_level, false /*download*/)?,
                env::current_dir().unwrap(),
                target.as_ref().map(String::as_str),
            )
        }
    }
}

fn run_esp_idf_menuconfig<'a>(
    pio: Pio,
    project: impl AsRef<Path>,
    target: Option<&'a str>,
) -> Result<()> {
    let project = project.as_ref();

    let platformio_ini = project.join("platformio.ini");

    if platformio_ini.exists() && platformio_ini.is_file() {
        // We are configuring a Pio-first project (possibly a PIO->Cargo one)
        // Just open up the PlatformIO->ESPIDF menuconfig system. It should work out of the box
        info!("Found platformio.ini in {}", project.display());

        pio.run_with_args(&["-t", "menuconfig"])
    } else {
        info!(
            "platformio.ini not found in {}, assuming a Cargo-first project",
            project.display()
        );

        let target = if let Some(target) = target {
            info!("Using explicitly passed target {}", target);

            target.to_owned()
        } else {
            let target = cargo::Crate::new(project).scan_config_toml(|value| {
                value
                    .get("build")
                    .map(|table| table.get("target"))
                    .flatten()
                    .map(|value| value.as_str())
                    .flatten()
                    .map(|str| str.to_owned())
            })?;

            if target.is_none() {
                bail!("Cannot find 'target=' specification in any Cargo configuration file. Please use the --target parameter to specify the target on the command line");
            }

            let target = target.unwrap();

            info!("Using pre-configured target {}", target);

            target
        };

        let resolution = Resolver::new(pio.clone())
            .params(ResolutionParams {
                platform: Some("espressif32".into()),
                frameworks: vec!["espidf".into()],
                target: Some(target),
                ..Default::default()
            })
            .resolve(true)?;

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

                fs::copy(&sdkconfig, &dest_sdkconfig)?;
            }
        }

        let current_dir = env::current_dir()?;

        env::set_current_dir(&project_path)?;

        let status = pio.run_with_args(&["-t", "menuconfig"]);

        env::set_current_dir(current_dir)?;

        status?;

        for sdkconfig in sdkconfigs {
            let dest_sdkconfig = project_path.join(sdkconfig.file_name().unwrap());

            if dest_sdkconfig.exists() {
                fs::copy(dest_sdkconfig, sdkconfig)?;
            }
        }

        Ok(())
    }
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
