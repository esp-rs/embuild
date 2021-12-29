use log::LevelFilter;
use structopt::clap::AppSettings;
use structopt::StructOpt;

mod build;
mod menuconfig;

#[derive(StructOpt)]
#[structopt(global_setting = AppSettings::GlobalVersion)]
#[structopt(bin_name = "cargo")]
struct Opts {
    #[structopt(subcommand)]
    sub_cmd: CargoSubCommand,
}

#[derive(StructOpt)]
enum CargoSubCommand {
    Idf(CargoIdfOpts),
}

#[derive(StructOpt)]
enum CargoIdfOpts {
    Menuconfig(menuconfig::MenuconfigOpts),
    Flash,
    EraseFlash,
    Monitor,
    Size,
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

    let CargoSubCommand::Idf(opts) = Opts::from_args().sub_cmd;
    match opts {
        CargoIdfOpts::Menuconfig(opts) => menuconfig::run(opts)?,
        _ => unimplemented!(),
    };

    Ok(())
}
