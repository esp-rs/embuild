# Build support for embedded Rust

[![CI](https://github.com/esp-rs/embuild/actions/workflows/ci.yml/badge.svg)](https://github.com/esp-rs/embuild/actions/workflows/ci.yml)
![crates.io](https://img.shields.io/crates/v/embuild.svg)
[![docs.rs](https://img.shields.io/docsrs/embuild)](https://docs.rs/embuild/latest/embuild/)
[![Matrix](https://img.shields.io/matrix/esp-rs:matrix.org?label=join%20matrix&color=BEC5C9&logo=matrix)](https://matrix.to/#/#esp-rs:matrix.org)

A library with many utilities for building embedded frameworks, libraries, and other
artifacts in a cargo build script.

It is currently mainly used to simplify building the
[`esp-idf`](https://github.com/espressif/esp-idf) in the build script of the
[`esp-idf-sys`](https://github.com/esp-rs/esp-idf-sys) crate, but anyone may use these
utilities as they're intended to be general. They're organized into specific modules so
that they and their dependencies can be turned on or off with features.

A list of current features and their utilities:
- `pio`
    - Platformio support.
- `cmake`
    - CMake file-api support and utilities.
- `glob` (used in the `build` module)
    - Glob utilities.
- `manifest` (used in the `cargo` module)
    - Cargo.toml and config.toml utilities.
- `espidf`
    - An installer to install the esp-idf framework.
- `git`
    - Git utilities for manipulating repositories using the git CLI.
- `kconfig`
    - kconfig file parsing.
- `elf` (`bingen`, `symgen` and `espidf::ulp_fsm` modules)
    - Elf file manipulation.

Other utilities that are not behind features include:
- `cargo`
    - Utils for interacting with cargo through the CLI, and stdout in a build script.
- `cmd`
    - Macros and wrappers for running commands and getting their results easier.
- `cli`
    - Command line arguments manipulation.

## Tools

This repository also provides two CLI tools:

- [`cargo-pio`](cargo-pio)
- [`ldproxy`](ldproxy)
- [`cargo-idf`](cargo-idf)