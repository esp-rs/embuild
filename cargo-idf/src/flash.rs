use std::ffi::OsStr;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::bail;
use clap::{AppSettings, ArgEnum, Args};
use strum::{Display, EnumString};

use crate::build;

#[derive(Args)]
#[clap(global_setting = AppSettings::DisableVersionFlag)]
pub struct FlashOpts {
    /// Which bootloader to flash [possible values: esp-idf, none, <file>]
    ///
    /// - `esp-idf` will flash the bootloader compiled locally from the esp-idf.
    /// - `none` prevents flashing a bootloader.
    /// - `<file>` will flash the user provided binary file if it exists.
    #[clap(
        long,
        default_value_t = Bootloader::EspIdf,
        parse(try_from_os_str = Bootloader::try_from_os_str),
        verbatim_doc_comment
    )]
    bootloader: Bootloader,

    /// How to flash the binary
    #[clap(long, arg_enum, default_value_t = Mode::Esptool)]
    mode: Mode,

    #[clap(flatten)]
    build_opts: build::BuildOpts,
}

#[derive(Debug, ArgEnum, Clone, Copy)]
pub enum Mode {
    Esptool,
    Dfu,
    Uf2,
}

#[derive(Debug, Clone, EnumString, Display)]
#[strum(serialize_all = "kebab-case")]
pub enum Bootloader {
    EspIdf,
    None,
    #[strum(default)]
    #[strum(to_string = "<file>")]
    File(PathBuf),
}

impl Bootloader {
    pub fn try_from_os_str(arg: &OsStr) -> Result<Bootloader, anyhow::Error> {
        let val = if let Some(arg) = arg.to_str() {
            Bootloader::from_str(arg).unwrap()
        } else {
            Bootloader::File(arg.into())
        };

        if let Bootloader::File(ref path) = val {
            if !path.is_file() {
                bail!("'{}' is not a file", path.display())
            }
        }
        Ok(val)
    }
}

pub fn run(opts: FlashOpts) -> anyhow::Result<()> {
    Ok(())
}
