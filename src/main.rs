use std::{env, fs};

use anyhow::*;

use build::{CopyFiles, Files};
use pio::*;

fn main() -> Result<()> {
    env_logger::init_from_env(env_logger::Env::default()
        .filter_or("MY_LOG_LEVEL", "info"));

    //let pio_dir = tempdir()?;
    let pio_dir = env::current_dir()?.join("..").join("pio-install");

    if !pio_dir.exists() {
        fs::create_dir(&pio_dir)?;
    }

    let mut pio_installer = PioInstaller::new()?;

    pio_installer.pio(&pio_dir);

    let pio = pio_installer.update()?;

    println!("{:?}", &pio);

    let mut builder: build::Builder = Default::default();

    builder.pio(pio);

    //let project_dir = tempdir()?;
    let project_dir = env::current_dir()?.join("..").join("pio-build");

    if !project_dir.exists() {
        fs::create_dir(&project_dir)?;
    }

    // builder
    //     .pio_project_dir(&project_dir)
    //     .framework("espidf")
    //     .mcu("ESP32")
    //     .link()
    //     .bindgen(
    //         env::current_dir()?.join("src").join("idf-target").join("esp32").join("bindings.h"),
    //         env::current_dir()?.join("bindings.rs"))
    //     .copy_files(CopyFiles::Main(Files {
    //         files: vec![env::current_dir()?.join("src").join("idf-target").join("esp32").join("sdkconfig.defaults")],
    //         dest_dir: ".".into(),
    //         symlink: true,
    //     }))
    //     //.build_flags(format!("-I{}", project_dir))
    //     .run()?;

    builder
        .pio_project_dir(&project_dir)
        .framework("espidf")
        .mcu("ESP32")
        // .link()
        // .bindgen(
        //     env::current_dir()?.join("src").join("tft.h"),
        //     env::current_dir()?.join("tft.rs"))
        .framework("arduino")
        .platform("espressif32")
        .library("TFT_eSPI")
        // .library("lvgl")
        // .unchecked_library("SPI")
        // .unchecked_library("FS")
        // .unchecked_library("SPIFFS")
        .run()?;

    // builder
    //     // .framework("esp8266-rtos-sdk")
    //     // .mcu("ESP8266")
    //     // .bindgen(
    //     //     env::current_dir()?.join("src").join("idf-target").join("esp8266").join("bindings.h"),
    //     //     env::current_dir()?.join("bindings.rs"))
    //     //.framework("arduino")
    //     //.platform("espressif32")
    //     //.library("TFT_eSPI")
    //     // .unchecked_library("SPI")
    //     // .unchecked_library("FS")
    //     // .unchecked_library("SPIFFS")
    //     .run()?;

    Ok(())
}
