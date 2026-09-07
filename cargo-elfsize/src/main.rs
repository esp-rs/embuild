//! Flash and RAM footprint reports for ELF firmware. Meant for CI size tracking:
//! `measure` each build (optionally against its base build) into a JSON file, then
//! `report` all of them as one markdown document.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use embuild::elfsize::{Measurement, Report, Sizes, DEFAULT_RAM_SECTIONS};

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo")]
enum Cargo {
    Elfsize(Args),
}

/// Flash and RAM footprint reports for ELF firmware
#[derive(clap::Args)]
#[command(version, arg_required_else_help = true)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure an ELF file, optionally against a base build of it, into a JSON measurement
    Measure {
        /// Label of the build in the report, e.g. the example and the chip
        #[arg(long)]
        label: Option<String>,

        /// Section name prefixes counted as RAM
        #[arg(long, value_delimiter = ',', value_name = "PREFIX,...", default_values_t = DEFAULT_RAM_SECTIONS.iter().map(|s| s.to_string()))]
        ram: Vec<String>,

        /// Write the measurement to this file rather than to stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// The ELF files: <NEW> alone, or <BASE> <NEW> to diff the two
        #[arg(required = true, num_args = 1..=2, value_name = "ELF")]
        files: Vec<PathBuf>,
    },

    /// Render measurements as one markdown report
    Report {
        /// Report title
        #[arg(long)]
        title: Option<String>,

        /// Print a GitHub `::warning::` line for each region that grew by more than
        /// PERCENT, and list those regions first in the report
        #[arg(long, value_name = "PERCENT")]
        warn: Option<f64>,

        /// The JSON measurements, in report order
        #[arg(required = true, value_name = "JSON")]
        files: Vec<PathBuf>,
    },
}

fn main() -> Result<()> {
    let Cargo::Elfsize(args) = Cargo::parse();

    match args.command {
        Command::Measure {
            label,
            ram,
            output,
            files,
        } => measure(label, &ram, output, &files),
        Command::Report { title, warn, files } => report(title, warn, &files),
    }
}

fn measure(
    label: Option<String>,
    ram: &[String],
    output: Option<PathBuf>,
    files: &[PathBuf],
) -> Result<()> {
    let ram: Vec<&str> = ram.iter().map(String::as_str).collect();

    let (base, new) = match files {
        [new] => (None, new),
        [base, new] => (Some(base), new),
        _ => unreachable!("clap limits the file count to 1 or 2"),
    };

    let measurement = Measurement {
        label: label.unwrap_or_else(|| {
            new.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        }),
        base: base.map(|base| Sizes::from_file(base, &ram)).transpose()?,
        new: Sizes::from_file(new, &ram)?,
    };

    let json = serde_json::to_string_pretty(&measurement)?;

    match output {
        Some(output) => fs::write(&output, json)
            .with_context(|| format!("Cannot write {}", output.display()))?,
        None => println!("{json}"),
    }

    Ok(())
}

fn report(title: Option<String>, warn: Option<f64>, files: &[PathBuf]) -> Result<()> {
    let measurements = files
        .iter()
        .map(|file| {
            let json = fs::read_to_string(file)
                .with_context(|| format!("Cannot read {}", file.display()))?;
            serde_json::from_str::<Measurement>(&json)
                .with_context(|| format!("Not a measurement: {}", file.display()))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut report = Report::new(&measurements);

    if let Some(title) = &title {
        report = report.title(title);
    }

    if let Some(warn) = warn {
        report = report.warn(warn);
    }

    print!("{}", report.markdown());

    for i in report.increases() {
        println!(
            "::warning title={}::{} grew by {:.2}% ({} -> {} bytes)",
            i.label, i.region, i.percent, i.base, i.new
        );
    }

    Ok(())
}
