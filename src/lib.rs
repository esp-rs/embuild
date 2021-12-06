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

pub mod bingen;
pub mod build;
pub mod cargo;
pub mod cli;
pub mod fs;
pub mod kconfig;
pub mod python;
pub mod symgen;
pub mod utils;

// This needs to be exported because some macros use `anyhow` and we don't want to force
// an explicit dependency on the user.
#[doc(hidden)]
pub use anyhow;
#[cfg(feature = "which")]
pub use which;
