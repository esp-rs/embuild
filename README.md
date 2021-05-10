# cargo-pio = [Cargo](https://doc.rust-lang.org/cargo/) + [PlatformIO](https://platformio.org/)

**Build Rust embedded projects with PlatformIO!**

## Why?

If you are building a mixed Rust/C project, or a pure Rust project that needs to call into the Vendor SDKs for your board, cargo-pio might help you.

Benefits:
* Cargo integrated in the PlatformIO build. Use PlatformIO & VSCode to drive your firmware build/upload workflow as if you are coding in C/C++. Cargo will be used transparently for your Rust code
* No need to download & install vendor GCC tollchains or SDKs. All handled by PlatformIO
* Using C/C++ libraries published in the PlatformIO [registry](https://platformio.org/lib) works too*

 *NOTE: you might still want to use [Bindgen](https://rust-lang.github.io/rust-bindgen/) to generate Rust bindings for those. Check [esp-idf-sys](https://github.com/ivmarkov/esp-idf-sys) for an example Rust bindings' crate that has integration with cargo-pio.

## Quickstart
* cargo-pio is a Cargo plug-in. Install with<br>`cargo install --git https://github.com/ivmarkov/cargo-pio.git cargo-pio`
* Download and install/upgrade PlatformIO:<br>`cargo pio installpio`
* Create a new Cargo/PIO project:<br>`cargo pio new --board <your-board> <path-to-your-new-project>`
* Enter your newly-generated project:<br>`cd <path-to-your-new-project>`
* Build it:<br>`cargo pio build [--release]`
  * Or `cargo pio run -e debug` which is equivalent
  * Or even just `pio run -e debug` - that is - if PlatformIO is on your `$PATH`. Once the Cargo/PIO project is generated, you don't actually need cargo-pio to build it!

## How it works
* cargo-pio generates `Cargo.py` - a special PlatformIO custom build script that hooks into your `platformio.ini` and calls Cargo to incrementally build your Rust code.
* As per above, once you create a Cargo/PIO initial project, you don't actually need cargo-pio for building it - it is all standard PlatformIO build - calling Cargo for your Rust code automatically!

## More details
* Your Rust code needs to be in a library crate of type `staticlib`
* Easiest to create the project structure with `cargo pio new ...` as per above
  * It will create the correct Cargo library crate and most importantly, it will correctly hook `Cargo.py` and your Rust library crate in `platformio.ini`
  * Examine the generated project structure to get an idea of what is possible
* This crate can of course depend on and use other Rust crates, provided that those compile under your embedded target
* Call `cargo pio --help` and examine the various subcommands and options
