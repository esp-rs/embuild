//! Report the flash and RAM footprint of an ELF executable, optionally as a diff against
//! a base build. Meant for CI size tracking; the output is markdown.

use std::env;
use std::process::exit;

use anyhow::{bail, Result};
use embuild::elfsize::{Report, Sizes, DEFAULT_RAM_SECTIONS};

const USAGE: &str = "\
Usage: cargo elfsize [OPTIONS] [<base.elf>] <new.elf>

Print the flash and RAM footprint of <new.elf> as a markdown table. With <base.elf>,
print the difference between the two instead.

Options:
  --title <TITLE>        Report title [default: Size report]
  --warn <PERCENT>       Also print a GitHub `::warning::` line for each region that
                         grew by more than PERCENT (needs <base.elf>)
  --ram <PREFIX,...>     Section name prefixes counted as RAM
                         [default: .data,.bss,.rwtext,.rwdata,.noinit,.trap]
  -h, --help             Print this help";

struct Args {
    title: String,
    warn: Option<f64>,
    ram: Vec<String>,
    base: Option<String>,
    new: String,
}

fn parse_args() -> Result<Args> {
    let mut title = "Size report".to_string();
    let mut warn = None;
    let mut ram = DEFAULT_RAM_SECTIONS.iter().map(|s| s.to_string()).collect();
    let mut files = Vec::new();

    // When run as `cargo elfsize`, cargo passes the subcommand name as the first argument
    let mut args = env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("elfsize") {
        args.next();
    }

    while let Some(arg) = args.next() {
        let mut value = |what: &str| match args.next() {
            Some(value) => Ok(value),
            None => bail!("{arg} needs a {what}\n\n{USAGE}"),
        };

        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                exit(0);
            }
            "--title" => title = value("title")?,
            "--warn" => warn = Some(value("percentage")?.parse()?),
            "--ram" => {
                ram = value("prefix list")?
                    .split(',')
                    .map(|s| s.to_string())
                    .collect()
            }
            _ if arg.starts_with('-') => bail!("Unknown option {arg}\n\n{USAGE}"),
            _ => files.push(arg),
        }
    }

    let (base, new) = match files.len() {
        1 => (None, files.remove(0)),
        2 => (Some(files.remove(0)), files.remove(0)),
        _ => bail!("{USAGE}"),
    };

    Ok(Args {
        title,
        warn,
        ram,
        base,
        new,
    })
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let ram: Vec<&str> = args.ram.iter().map(String::as_str).collect();

    let new = Sizes::from_file(&args.new, &ram)?;
    let base = match &args.base {
        Some(base) => Some(Sizes::from_file(base, &ram)?),
        None => None,
    };

    let mut report = Report::new(&args.title, &new);

    if let Some(base) = &base {
        report = report.base(base);
    }

    if let Some(warn) = args.warn {
        report = report.warn(warn);
    }

    print!("{}", report.markdown());

    for increase in report.increases() {
        println!(
            "::warning title={}::{} grew by {:.2}% ({} -> {} bytes)",
            args.title, increase.region, increase.percent, increase.base, increase.new
        );
    }

    Ok(())
}
