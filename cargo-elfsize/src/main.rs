//! Report the flash and RAM footprint of an ELF executable, optionally as a diff against
//! a base build. Meant for CI size tracking; the output is markdown.

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use embuild::elfsize::{Report, Sizes, DEFAULT_RAM_SECTIONS};

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    Elfsize(Args),
}

/// Print the flash and RAM footprint of an ELF executable as a markdown table, or the
/// difference between two builds of it
#[derive(clap::Args)]
#[command(version, arg_required_else_help = true)]
struct Args {
    /// Report title
    #[arg(long, default_value = "Size report")]
    title: String,

    /// Also print a GitHub `::warning::` line for each region that grew by more than
    /// PERCENT (needs <BASE>)
    #[arg(long, value_name = "PERCENT")]
    warn: Option<f64>,

    /// Section name prefixes counted as RAM
    #[arg(long, value_delimiter = ',', value_name = "PREFIX,...", default_values_t = DEFAULT_RAM_SECTIONS.iter().map(|s| s.to_string()))]
    ram: Vec<String>,

    /// The ELF files: <NEW> alone, or <BASE> <NEW> to diff the two
    #[arg(required = true, num_args = 1..=2, value_name = "ELF")]
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let Cargo::Elfsize(args) = Cargo::parse();

    let ram: Vec<&str> = args.ram.iter().map(String::as_str).collect();

    let (base, new) = match args.files.as_slice() {
        [new] => (None, new),
        [base, new] => (Some(base), new),
        _ => unreachable!("clap limits the file count to 1 or 2"),
    };

    let new = Sizes::from_file(new, &ram)?;
    let base = match base {
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
