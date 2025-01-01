# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Deprecated

### Breaking
- Module `espidf`: Support building with non-git repositories (#95)
- Module `espidf`: Provide tools exported env vars; do not assume that each installed tool is an executable binary
- Rename `BindgenExt::headers` to `Bindgen::path_headers` to avoid collision with the existing `bindgen::Builder::headers` method

### Added
- Add support for PlatformIO platform = native (#97)
- Re-export the `bindgen` crate as `embuild::bindgen::types` so that downstream crates can use it without having to add it as a dependency

### Fixed

## [0.32.0] - 2024-06-23
### Breaking
* bindgen: updated to the latest bindgen version. (#75)
* python: check_python_at_least() now returns a Result<PythonVersion> instead of Result<()>. (#85)
### Fixed
* git: speed up submodule git cloning by utilizing jobs. (#86)
* esp-idf: fix builds against idf >= v5.3 by introducing new export PATH logic. (#85)
* esp-idf: use correct overrides on platforms like MacOS for export PATH, etc. (#88)

## [0.31.4] - 2023-10-27
* PIO: Espressif MCUs esp32c2/c5/c6/h2 had a wrong Rust target assigned to them

## [0.31.3] - 2023-08-22
* New module, `espidf::sysenv` for easier propagation and consumption of the ESP IDF build settings of `esp-idf-sys`

## [0.31.2] - 2023-05-08
* Compatibility with PlatformIO 6.1

## [0.31.1] - 2023-03-20
* Compatibility with MacOS ARM64
* Generic notion of a GIT-based SDK (used by the `esp-idf-sys` bindings for the ESP IDF and by the `chip-sys` bindings for the Matter C++ SDK)

## [0.31.0] - 2022-12-09
* Bindgen dependency was bumped up to 0.63
