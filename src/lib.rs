// Allows docs.rs to document any needed features for items
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(feature = "bindgen")]
pub mod bindgen;

#[cfg(feature = "pio")]
pub mod pio;

#[cfg(feature = "cmake")]
pub mod cmake;

#[cfg(feature = "espidf")]
pub mod espidf;

#[cfg(feature = "git")]
pub mod git;

#[cfg(feature = "kconfig")]
pub mod kconfig;

#[cfg(feature = "elf")]
pub mod symgen;

#[cfg(feature = "elf")]
pub mod bingen;

pub mod build;
pub mod cargo;
pub mod cli;
pub mod cmd;
pub mod fs;
pub mod python;
pub mod utils;

#[cfg(feature = "which")]
pub use which;
