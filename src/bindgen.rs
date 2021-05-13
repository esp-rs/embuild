use std::{env, ffi::OsStr, os::unix::prelude::OsStrExt, path::PathBuf, process::Command};

use anyhow::*;

pub const VAR_PIO_BINDGEN_RUN: &'static str = "PIO_BINDGEN_RUN";
pub const VAR_PIO_BINDGEN_INC_FLAGS: &'static str = "PIO_BINDGEN_INC_FLAGS";
pub const VAR_PIO_BINDGEN_MCU: &'static str = "PIO_BINDGEN_MCU";
pub const VAR_PIO_BINDGEN_LINKER: &'static str = "PIO_BINDGEN_LINKER";

pub fn run(
        library_name: impl AsRef<str>,
        bindings_headers: &[impl AsRef<str>],
        llvm_target: impl AsRef<str> /* TODO: Can we get rid of this? */) -> Result<()> {
    run_raw(
        library_name,
        bindings_headers,
        llvm_target,
        &get_var(VAR_PIO_BINDGEN_INC_FLAGS)?.split(' ').map(str::to_string).collect::<Vec<_>>(),
        Some(get_var(VAR_PIO_BINDGEN_MCU)?),
        Some(get_var(VAR_PIO_BINDGEN_LINKER)?))
}

pub fn run_raw(
        library_name: impl AsRef<str>,
        bindings_headers: &[impl AsRef<str>],
        llvm_target: impl AsRef<str>,
        clang_args: &[impl AsRef<str>],
        mcu: Option<impl AsRef<str>>,
        linker: Option<impl AsRef<str>>) -> Result<()> {
    let library_prefix = library_name.as_ref().to_uppercase().replace('-', "_");

    let regenerate_var = format!("{}_REGENERATE", library_prefix);
    let bindings_file_var = format!("{}_BINDINGS_FILE", library_prefix);

    let regenerate = env::var(&regenerate_var).is_ok() || env::var(VAR_PIO_BINDGEN_RUN).is_ok();
    if !regenerate {
        if let Some(mcu) = mcu {
            let mcu = mcu.as_ref();

            println!(
                "cargo:warning=None of the environment variables {} or {} are defined.
                Using pre-generated bindings for MCU '{}'.",
                regenerate_var,
                VAR_PIO_BINDGEN_RUN,
                mcu);
            println!(
                "cargo:rustc-env={}=bindings_{}.rs",
                bindings_file_var,
                mcu);
        } else {
            println!(
                "cargo:warning=None of the environment variables {} or {} are defined.
                Using pre-generated bindings.",
                regenerate_var,
                VAR_PIO_BINDGEN_RUN);
            println!(
                "cargo:rustc-env={}=bindings.rs",
                bindings_file_var);
        }

        return Ok(());
    }

    // TODO: println!("cargo:rerun-if-changed={}/sdkconfig.h", idf_bindings_header_dir);

    let linker = if let Some(linker) = linker {
        linker.as_ref().to_owned()
    } else if let Ok(linker) = env::var("RUSTC_LINKER") {
        linker
    } else {
        bail!("No explicit linker, and env var RUSTC_LINKER not set either");
    };

    let sysroot = Command::new(linker)
        .arg("--print-sysroot")
        .output()
        .map(|mut output| {
            // Remove newline from end.
            output.stdout.pop();
            PathBuf::from(OsStr::from_bytes(&output.stdout))
                .canonicalize()
                .expect("failed to canonicalize sysroot")
        })
        .expect("failed getting sysroot");

    let mut bindings = bindgen::Builder::default()
        .use_core()
        .layout_tests(false)
        .rustfmt_bindings(true)
        .ctypes_prefix("c_types"/*"libc"*/)
        .derive_default(true)
        // TODO: Enable that and fix all call sites .default_enum_style(EnumVariation::Rust { non_exhaustive: false })
        .clang_arg(format!("--sysroot={}", sysroot.display()))
        .clang_arg(format!("-I{}/include", sysroot.display()))
        .clang_arg("-D__bindgen")
        .clang_args(&["-target", llvm_target.as_ref()])
        .clang_args(&["-x", "c"])
        .clang_args(clang_args);

    for header in bindings_headers {
        bindings = bindings.header(header.as_ref());
        println!("cargo:rerun-if-changed={}", header.as_ref());
    }

    eprintln!("Bindgen flags: {:?}", bindings.command_line_flags());

    let output_file = PathBuf::from(env::var("OUT_DIR")?).join("bindings.rs");
    eprintln!("Output: {:?}", &output_file);

    bindings.generate().expect("Failed to generate bindings").write_to_file(&output_file)?;

    // Run rustfmt on the generated bindings separately, because the xtensa custom toolchain does not have rustfmt yet
    // Hence why we need to use the rustfmt from the stable buildchain (where the assumption is, it is already installed)
    Command::new("rustup")
        .arg("run")
        .arg("stable")
        .arg("rustfmt")
        .arg(&output_file)
        .status()?;

    println!("cargo:rustc-env={}={}", bindings_file_var, output_file.display());

    Ok(())
}

fn get_var(var_name: &str) -> Result<String> {
    match env::var(var_name) {
        Err(_) => bail!("Cannot find env variable {}. Make sure you are bulding this crate with cargo-pio-generated support", var_name),
        Ok(value) => Ok(value),
    }
}
