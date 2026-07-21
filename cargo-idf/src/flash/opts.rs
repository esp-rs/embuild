use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::bail;
use clap::{AppSettings, ArgEnum, Args};
use embuild::utils::OsStrExt;
use strum::{Display, EnumDiscriminants, EnumString};

use crate::build;

#[derive(Args)]
#[clap(global_setting = AppSettings::DisableVersionFlag)]
pub struct FlashOpts {
    /// One or more images to flash [possible values: all, bootloader, partition-table,
    /// app, <partition name> <file>, <address> <file>]
    ///
    /// - `all`: flash the whole project (bootloader, partition-table, app)
    /// - `bootloader`: flash bootloader (see `--bootloader` option)
    /// - `partition-table`: flash the partition table (see `--partition-table` option)
    /// - `app`: flash the app
    /// - `<partition name> <file>`: flash <file> at the address of <partition name>
    /// - `<address> <file>`: flash <file> at <address>
    #[clap(
        parse(from_os_str = ImageArg::from_os_str),
        validator_os = ImageArg::parse_validator(),
        verbatim_doc_comment,
        default_value = "all"
    )]
    images: Vec<ImageArg>,

    /// The bootloader binary file to use instead of the default
    #[clap(long, parse(from_os_str), value_name = "file")]
    bootloader: Option<PathBuf>,

    /// The partition table `.csv` file to use instead of the default
    #[clap(long, parse(from_os_str), value_name = "file")]
    partition_table: Option<PathBuf>,

    #[clap(flatten)]
    build_opts: build::BuildOpts,
}

#[derive(Debug, ArgEnum, Clone, Copy)]
pub enum Mode {
    Esptool,
    Dfu,
    Uf2,
}

#[derive(Debug, Clone, Display, EnumDiscriminants)]
#[strum_discriminants(name(ImageArgKind))]
pub enum ImageArg {
    #[strum(to_string = "<name>")]
    Name(ImageName),
    #[strum(to_string = "<address>")]
    Address(usize),
    #[strum(to_string = "<partition name or file>")]
    PartitionOrFile(OsString),
    Partition(String),
    File(PathBuf),
}

impl Default for ImageArgKind {
    fn default() -> Self {
        ImageArgKind::Name
    }
}

#[derive(Debug, EnumString, Display, Clone, Copy)]
#[strum(serialize_all = "kebab-case")]
pub enum ImageName {
    All,
    Bootloader,
    PartitionTable,
    App,
}

impl ImageArg {
    fn from_os_str(arg: &OsStr) -> ImageArg {
        if let Some(arg) = arg.to_str() {
            if let Ok(name) = ImageName::from_str(arg) {
                return ImageArg::Name(name);
            } else if let Ok(address) = arg.parse::<usize>() {
                return ImageArg::Address(address);
            }
        }
        ImageArg::PartitionOrFile(arg.to_owned())
    }

    fn parse_validator() -> impl FnMut(&OsStr) -> Result<(), anyhow::Error> {
        let mut previous = ImageArgKind::default();
        move |arg| {
            let next = Self::from_os_str(arg);
            previous = Self::parse(previous, next)?.into();
            Ok(())
        }
    }

    /// Parses image arg with a given previous arg kind.
    ///
    /// Never returns [`ImageArg::PartitionOrFile`].
    pub fn parse(last: ImageArgKind, next: ImageArg) -> Result<ImageArg, anyhow::Error> {
        use ImageArgKind::*;
        let result = match (last, next) {
            (Name | File, ImageArg::PartitionOrFile(file)) => {
                ImageArg::Partition(file.try_to_str()?.into())
            }
            (Partition | Address, ImageArg::PartitionOrFile(file)) => ImageArg::File(file.into()),
            (Name | File, val @ ImageArg::Name(_) | val @ ImageArg::Address(_)) => val,
            (Partition | Address, _) => bail!("expected <file>"),

            (_, ImageArg::File(_) | ImageArg::Partition(_)) | (PartitionOrFile, _) => {
                unreachable!("invalid state")
            }
        };
        Ok(result)
    }
}
