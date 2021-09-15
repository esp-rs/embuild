use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::Command;
use std::vec::Vec;

use anyhow::*;
use embuild::build;
use embuild::cli::{ParseFrom, UnixCommandArgs};
use log::*;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::new()
            .write_style_or("LDPROXY_LOG_STYLE", "Auto")
            .filter_or("LDPROXY_LOG", LevelFilter::Info.to_string()),
    )
    .target(env_logger::Target::Stderr)
    .format_level(false)
    .format_indent(None)
    .format_module_path(false)
    .format_timestamp(None)
    .init();

    info!("Running ldproxy");

    debug!("Raw link arguments: {:?}", env::args());

    let mut args = args()?;

    debug!("Link arguments: {:?}", args);

    let [linker, remove_duplicate_libs, cwd] = [
        &build::LDPROXY_LINKER_ARG,
        &build::LDPROXY_DEDUP_LIBS_ARG,
        &build::LDPROXY_WORKING_DIRECTORY_ARG,
    ]
    .parse_from(&mut args);

    let linker = linker
        .ok()
        .and_then(|v| v.into_iter().last())
        .unwrap_or_else(|| {
            panic!(
                "Cannot locate argument '{}'",
                build::LDPROXY_LINKER_ARG.format(Some("<linker>"))
            )
        });

    debug!("Actual linker executable: {}", linker);

    let cwd = cwd.ok().and_then(|v| v.into_iter().last());
    let remove_duplicate_libs = remove_duplicate_libs.is_ok();

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

        for arg in args {
            if libs.contains_key(&arg) {
                *libs.get_mut(&arg).unwrap() -= 1;

                if libs[&arg] == 0 {
                    libs.remove(&arg);
                }
            }

            if !libs.contains_key(&arg) {
                deduped_args.push(arg);
            }
        }

        deduped_args
    } else {
        args
    };

    let mut cmd = Command::new(&linker);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.args(&args);

    debug!("Calling actual linker: {:?}", cmd);

    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    debug!("==============Linker stdout:\n{}\n==============", stdout);
    debug!("==============Linker stderr:\n{}\n==============", stderr);

    if !output.status.success() {
        bail!(
            "Linker {} failed: {}\nSTDERR OUTPUT:\n{}",
            linker,
            output.status,
            stderr
        );
    }

    if env::var("LDPROXY_LINK_FAIL").is_ok() {
        bail!("Failure requested");
    }

    Ok(())
}

/// Get all arguments
///
/// **Currently only supports gcc-like arguments**
///
/// FIXME: handle other linker flavors (https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-flavor)
fn args() -> Result<Vec<String>> {
    let mut result = Vec::new();

    for arg in env::args().skip(1) {
        // Rustc could invoke use with response file arguments, so we could get arguments
        // like: `@<link-args-file>` (as per `@file` section of
        // https://gcc.gnu.org/onlinedocs/gcc-11.2.0/gcc/Overall-Options.html)
        //
        // Deal with that
        if let Some(arg) = arg.strip_prefix('@') {
            let rsp_file = Path::new(arg);
            // get all arguments from the response file if it exists
            if rsp_file.exists() {
                let contents = std::fs::read_to_string(rsp_file)?;
                debug!("Contents of {}: {}", arg, contents);

                result.extend(UnixCommandArgs::new(&contents));
            }
            // otherwise just add the argument as normal
            else {
                result.push(arg.to_owned());
            }
        } else {
            result.push(arg);
        }
    }

    Ok(result)
}
