use clap::Subcommand;
use log::LevelFilter;
use clap::AppSettings;
use clap::Parser;

mod build;
mod menuconfig;
mod flash;

#[derive(Parser)]
#[clap(global_setting = AppSettings::PropagateVersion)]
#[clap(global_setting = AppSettings::DeriveDisplayOrder)]
#[clap(version)]
#[clap(bin_name = "cargo")]
struct Opts {
    #[clap(subcommand)]
    sub_cmd: CargoSubCommand,
}

#[derive(Subcommand)]
enum CargoSubCommand {
    #[clap(subcommand)]
    Idf(CargoIdfOpts),
}

#[derive(Subcommand)]
enum CargoIdfOpts {
    Menuconfig(menuconfig::MenuconfigOpts),
    Flash(flash::FlashOpts),
    Monitor,
    Size,
    EraseFlash,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("CARGO_IDF_LOG_STYLE", "Auto")
            .filter_or("CARGO_IDF_LOG", LevelFilter::Info.to_string()),
    )
    .target(env_logger::Target::Stderr)
    .format_indent(None)
    .format_module_path(false)
    .format_timestamp(None)
    .init();

    let CargoSubCommand::Idf(opts) = Opts::parse().sub_cmd;
    match opts {
        CargoIdfOpts::Menuconfig(opts) => menuconfig::run(opts)?,
        CargoIdfOpts::Flash(opts) => flash::run(opts)?,
        _ => unimplemented!(),
    };

    Ok(())
}
