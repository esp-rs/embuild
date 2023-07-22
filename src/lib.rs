//! # Build support for embedded Rust
//!
//! A library with many utilities for building embedded frameworks, libraries, and other
//! artifacts in a cargo build script.
//!
//! It is currently mainly used to simplify building the [`esp-idf`](https://github.com/espressif/esp-idf) in the build script of the
//! [`esp-idf-sys`](https://github.com/esp-rs/esp-idf-sys) crate, but anyone may use them as they're intended to be general. The
//! utilities are organized into specific modules so that they and their dependencies can be
//! turned on or off with features.

// Allows docs.rs to document any needed features for items (needs nightly rust).
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
