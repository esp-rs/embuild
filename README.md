# Build support for embedded Rust

![CI](https://github.com/esp-rs/embuild/actions/workflows/ci.yml/badge.svg)

A library with many utilities for building embedded frameworks, libraries, and other
artifacts in a cargo build script.

It is currently mainly used to simplify building the [`esp-idf`](https://github.com/espressif/esp-idf) in the build script of the
[`esp-idf-sys`](https://github.com/esp-rs/esp-idf-sys) crate, but anyone may use them as they're intended to be general. The
utilities are organized into specific modules so that they and their dependencies can be
turned on or off with features.

The follwing is the current list of features and their utilities:

- `pio = ["ureq", "bindgen", "tempfile", "which", "manifest", "serde", "serde_json"]`
    - Platformio support.
- `cmake = ["dep-cmake", "tempfile", "bindgen", "serde", "serde_json", "strum"]`
    - CMake file-api support and utilities.
- `glob = ["globwalk"]`
    - Glob utilities.
- `manifest = ["cargo_toml", "toml"]`
    - Cargo.toml and config.toml utilities.
- `espidf = ["tempfile", "which", "git", "serde", "serde_json", "strum", "dirs"]`
    - An installer to install the esp-idf framework.
- `git = ["remove_dir_all"]`
    - Git utilities for manipulating repositories using the git CLI.
- `kconfig = ["serde", "serde_json"]`
    - kconfig file parsing.
- `elf = ["xmas-elf"]`
    - Elf file manipulation.

Other utilities that are not behind features include:
- `cargo`
    - Utils for interacting with cargo through the CLI, and stdout in a build script.
- `cmd`
    - Macros and wrappers for running commands and getting their results easier.

## Tools

This repository also provides two CLI tools:

- [`cargo-pio`](cargo-pio)
- [`ldproxy`](ldproxy)
