# Build support for embedded Rust

![CI](https://github.com/esp-rs/embuild/actions/workflows/ci.yml/badge.svg)

Embuild crate offers cargo integration with other embedded build ecosystems & tools. Embuild is capable of:
- Generating bindings for the SDK, [ESP-IDF](https://github.com/espressif/esp-idf).
- Installing and configuring the IDF tools.
- Building [ESP-IDF](https://github.com/espressif/esp-idf).
- Generating linker flags for the Rust linking process of the binary crate.
Embuild is only required for applications using the [standard library (`std`)](https://esp-rs.github.io/book/overview/using-the-standard-library.html)

There are two ways of building with `embuild`:
- `native` **[Default]**: ESP-IDF build without external dependencies
- `pio`: Used initially now we try to avoid this method and is expected to be deprecated in the future since it adds a dependencies en PlatformIO

## Tools

- [`cargo-pio`](cargo-pio): Builds Rust embedded projects with PlatformIO!
    - `cargo-pio` helps calling the Vendor SDK ([ESP-IDF](https://github.com/espressif/esp-idf) in our case)
- [`ldproxy`](ldproxy): Tool to forward linker arguments to the actual linker executable also given as an argument to `ldproxy`.
    - *Currently, only gcc [linker flavor](https://doc.rust-lang.org/rustc/codegen-options/index.html#linker-flavor) is supported.*
