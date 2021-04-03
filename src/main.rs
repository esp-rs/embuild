use std::{env, fs};

use anyhow::*;

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

    builder.project(&project_dir);

    builder
        .framework("espidf")
        .framework("arduino")
        .platform("espressif32")
        .library("TFT_eSPI")
        // .unchecked_library("SPI")
        // .unchecked_library("FS")
        // .unchecked_library("SPIFFS")
        .run()?;

    // PioCommand::new(&pio, &project_dir)?
    //     .platform("espressif32")
    //     // .framework("arduino, espidf")
    //     .framework("arduino")
    //     .board("nodemcu-32s")
    //     .library("TFT_eSPI")
    //     // .platformini_opt(
    //     //     "platform_packages",
    //     //     "framework-arduinoespressif32 @ https://github.com/espressif/arduino-esp32.git#idf-release/v4.0")
    //     .run()?;

    Ok(())
}
