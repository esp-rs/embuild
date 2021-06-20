use std::{collections::HashMap, env, process::Command};

use anyhow::*;
use log::*;

const CMD_PIO_LINK: &'static str = "pio-link";

fn main() -> Result<()> {
    let level = LevelFilter::Info; // For now; make it a parameter

    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("CARGO_PIO_LOG_STYLE", "Auto")
            .filter_or(
                "CARGO_PIO_LOG",
                level.to_string()))
        .target(env_logger::Target::Stderr)
        .format_level(false)
        .format_indent(None)
        .format_module_path(false)
        .format_timestamp(None)
        .init();

    let mut args = env::args();
    args.next(); // Skip over the command-line executable

    run(args.next() == Some(CMD_PIO_LINK.to_owned()))
}

fn run(as_plugin: bool) -> Result<()> {
    info!("Running the cargo-pio-link linker wrapper");

    let linker = args(as_plugin)
        .find(|arg| arg.starts_with(pio::CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX))
        .map(|arg| arg[pio::CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX.len()..].to_owned())
        .unwrap();

    debug!("Actual linker executable: {}", linker);

    let remove_duplicate_libs = args(as_plugin)
        .find(|arg| arg == pio::CARGO_PIO_LINK_REMOVE_DUPLICATE_LIBS_ARG)
        .is_some();

    let args = if remove_duplicate_libs {
        debug!("Duplicate libs removal requested");

        let mut libs = HashMap::<String, usize>::new();

        for arg in args(as_plugin) {
            if arg.starts_with("-l") {
                *libs.entry(arg).or_default() += 1;
            }
        }

        debug!("Libs occurances: {:?}", libs);

        let mut deduped_args = Vec::new();

        for arg in args(as_plugin) {
            if libs.contains_key(&arg) {
                *libs.get_mut(&arg).unwrap() -= 1;

                if libs[&arg] == 0 {
                    libs.remove(&arg);
                }
            }

            if !libs.contains_key(&arg) && !arg.starts_with(pio::CARGO_PIO_LINK_ARG_PREFIX) {
                deduped_args.push(arg);
            }
        }

        deduped_args
    } else {
        args(as_plugin)
            .filter(|arg| !arg.starts_with(pio::CARGO_PIO_LINK_ARG_PREFIX))
            .collect()
    };

    let mut cmd = Command::new(linker);

    cmd.args(&args);
    cmd.status()?;

    Ok(())
}

fn args(as_plugin: bool) -> env::Args {
    let mut args = env::args();

    args.next(); // Skip over the command-line executable

    if as_plugin {
        args.next();
    }

    args
}
