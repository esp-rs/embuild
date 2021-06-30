use std::{collections::HashMap, env, fs, path::PathBuf, process::Command, vec::Vec};

use anyhow::*;
use log::*;

const CMD_PIO_LINK: &'static str = "pio-link";

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("CARGO_PIO_LOG_STYLE", "Auto")
            .filter_or(
                "CARGO_PIO_LOG",
                LevelFilter::Info.to_string()))
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

    debug!("Running as plugin: {}", as_plugin);
    debug!("Raw link arguments: {:?}", raw_args(as_plugin).collect::<Vec<String>>());

    let args = args(as_plugin)?;

    debug!("Link arguments: {:?}", args);

    let linker = args.iter()
        .find(|arg| arg.starts_with(pio::CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX))
        .map(|arg| arg[pio::CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX.len()..].to_owned())
        .expect(format!("Cannot locate argument {}", pio::CARGO_PIO_LINK_LINK_BINARY_ARG_PREFIX).as_str());

    debug!("Actual linker executable: {}", linker);

    let remove_duplicate_libs = args.iter()
        .find(|arg| arg.as_str() == pio::CARGO_PIO_LINK_REMOVE_DUPLICATE_LIBS_ARG)
        .is_some();

    let args = if remove_duplicate_libs {
        debug!("Duplicate libs removal requested");

        let mut libs = HashMap::<String, usize>::new();

        for arg in &args {
            if arg.starts_with("-l") {
                *libs.entry(arg.clone()).or_default() += 1;
            }
        }

        debug!("Libs occurances: {:?}", libs);

        let mut deduped_args = Vec::new();

        for arg in &args {
            if libs.contains_key(arg) {
                *libs.get_mut(arg).unwrap() -= 1;

                if libs[arg] == 0 {
                    libs.remove(arg);
                }
            }

            if !libs.contains_key(arg) && !arg.starts_with(pio::CARGO_PIO_LINK_ARG_PREFIX) {
                deduped_args.push(arg.clone());
            }
        }

        deduped_args
    } else {
        args.into_iter()
            .filter(|arg| !arg.starts_with(pio::CARGO_PIO_LINK_ARG_PREFIX))
            .collect()
    };

    let mut cmd = Command::new(linker);

    cmd.args(&args);
    cmd.status()?;

    Ok(())
}

fn args(as_plugin: bool) -> Result<Vec<String>> {
    let mut result = Vec::new();

    for arg in raw_args(as_plugin) {
        #[cfg(windows)]
        {
            // Apparently on Windows rustc thinks that it is dealing with LINK.EXE (even though it is running a custom toolchain where the linker is described as having a "gcc" flavor!)
            // Therefore, what we get there is this: 'cargo-pio-link @<link-args-file> (as per https://docs.microsoft.com/en-us/cpp/build/reference/linking?view=msvc-160)
            //
            // Deal with that
            if arg.starts_with("@") {
                let data = String::from_utf8(fs::read(PathBuf::from(&arg[1..]))?)?;

                for sub_arg in data.split_ascii_whitespace() {
                    result.push(sub_arg.into());
                }
            } else {
                result.push(arg);
            }
        }

        #[cfg(not(windows))]
        {
            result.push(arg);
        }
    }

    Ok(result)
}

fn raw_args(as_plugin: bool) -> env::Args {
    let mut args = env::args();

    args.next(); // Skip over the command-line executable

    if as_plugin {
        args.next();
    }

    args
}
