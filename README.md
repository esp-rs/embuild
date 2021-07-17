# cargo-pio = [Cargo](https://doc.rust-lang.org/cargo/) + [PlatformIO](https://platformio.org/)

**Build Rust embedded projects with PlatformIO!**

cargo-pio is a Cargo subcommand `cargo pio`, as well as a library crate.

## Why?

If you are building a mixed Rust/C project, or a pure Rust project that needs to call into the Vendor SDKs for your board, cargo-pio might help you.

## PlatformIO-first build

In this mode of operation, your embedded project would be a PlatformIO project **and** a Rust static library crate:
* The project build is triggered with PlatformIO (e.g. `pio run -t debug` or `cargo pio build`), and PlatformIO calls into Cargo to build the Rust library crate;
* `cargo-pio` is used as a Cargo subcommand to create the layout of the project;
* Such projects are called 'PIO->Cargo projects' in the `cargo-pio` help

Example:
* ```cargo install cargo-pio``` (or ```cargo install --git https://github.com/ivmarkov/cargo-pio.git cargo-pio```)
* ```cargo pio installpio```
* Create a new Cargo/PIO project:
  * ```cargo pio new --board <your-board> <path-to-your-new-project>```
* Enter your newly-generated project:
  * ```cd <path-to-your-new-project>```
* Build in debug:
  * ```pio run -t debug``` or ```cargo pio build```
* Build in release:
  * ```pio run -t release``` or ```cargo pio build --release```
* Note that once PlatformIO is installed and the PIO->Cargo project is created, you don't really need `cargo-pio`!

Call ```cargo pio --help``` to learn more about the various commands supported by `cargo-pio`.

## Cargo-first build

* In this mode of operation, your embedded project is a **pure Cargo project and PlatformIO does not get in the way**!
* `cargo-pio` is used as a library crate and driven programmatically from `build.rs` scripts.

Cargo-first builds however are less flexible. They are only suitable for building and (linking with) the "SYS" crate that represents the Vendor SDK for your board.
If you depend on other C libraries, you should be using a PlatformIO-first a.k.a. 'PIO->Cargo' project.

Example:
* Check the [esp-idf-sys](https://github.com/ivmarkov/esp-idf-sys) SYS crate (used by the [rust-esp32-std-hello](https://github.com/ivmarkov/rust-esp32-std-hello) binary crate). It demonstrates:
  * Automatic download and installation the SDK (ESP-IDF in that case), by programmatically driving PlatformIO;
  * Automatic generation of unsafe bindings with Bindgen for the ESP-IDF SDK. Crates like [esp-idf-hal](https://github.com/ivmarkov/esp-idf-hal) and [esp-idf-svc](https://github.com/ivmarkov/esp-idf-svc) depend on these bindings to implement higher level type-safe Rust abstractions;
  * Automatic generation of Rust link flags for the Rust Linker so that the ESP-IDF SDK is transparently linked into your binary Rust crate that you'll flash.
