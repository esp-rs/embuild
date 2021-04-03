use std::{collections::HashMap, env, fs, io::Write, path::PathBuf};

use anyhow::*;
use log::{info, trace};

use crate::{Board, Library, Pio, PioInstaller};

#[derive(Default, Clone, Debug)]
pub struct Builder {
    pio: Option<Pio>,
    platform: Option<String>,
    frameworks: Vec<String>,
    libraries: Vec<String>,
    unchecked_libraries: Vec<String>,
    board: Option<String>,
    bindgen: Option<bool>,
    link: Option<bool>,
    project: Option<PathBuf>,
}

impl Builder {
    pub fn pio(&mut self, pio: Pio) -> &mut Self {
        self.pio = Some(pio);
        self
    }

    pub fn board(&mut self, board: impl Into<String>) -> &mut Self {
        self.board = Some(board.into());
        self
    }

    pub fn platform(&mut self, platform: impl Into<String>) -> &mut Self {
        self.platform = Some(platform.into());
        self
    }

    pub fn framework(&mut self, framework: impl Into<String>) -> &mut Self {
        self.frameworks.push(framework.into());
        self
    }

    pub fn library(&mut self, library: impl Into<String>) -> &mut Self {
        self.libraries.push(library.into());
        self
    }

    pub fn unchecked_library(&mut self, library: impl Into<String>) -> &mut Self {
        self.unchecked_libraries.push(library.into());
        self
    }

    pub fn project(&mut self, project: impl Into<PathBuf>) -> &mut Self {
        self.project = Some(project.into());
        self
    }

    pub fn bindgen(&mut self, bindings_file: &str, includes_file: &str) -> &mut Self {
        self
    }

    pub fn link(&mut self) -> &mut Self {
        self
    }

    pub fn run(&mut self) -> Result<()> {
        self.resolve()?;

        let mut cmd = self.pio.as_ref().unwrap()
            .project(self.project.as_ref().unwrap());

        cmd.arg("run").status()?;

        Ok(())
    }

    pub fn resolve(&mut self) -> Result<()> {
        self.resolve_pio()?;
        self.resolve_framework()?;
        self.resolve_libraries()?;
        self.resolve_project()
    }

    fn resolve_pio(&mut self) -> Result<()> {
        if self.pio.is_none() {
            let installer = PioInstaller::new()?;
            self.pio = Some(installer.update()?);
        }

        info!("Resolved PlatformIO installation: {:?}", self.pio.as_ref().unwrap().core_dir.as_os_str());

        Ok(())
    }

    fn resolve_framework(&mut self) -> Result<()> {
        if self.board.is_some() {
            self.resolve_framework_by_board()?;
        } else {
            self.resolve_framework_all()?;
        }

        info!("Resolved platform: '{}', frameworks: [{}]", self.platform.as_ref().unwrap(), self.frameworks.join(", "));

        Ok(())
    }

    fn resolve_libraries(&mut self) -> Result<()> {
        if !self.libraries.is_empty() {
            let libraries_map = self.pio.as_ref().unwrap()
                .libraries(&self.libraries)?
                .into_iter()
                .map(|l| (l.name.clone(), l))
                .collect::<HashMap<String, Library>>();

            for lib_name in &self.libraries {
                let library = libraries_map.get(lib_name.as_str());

                if let Some(library) = library {
                    let configured_platform = self.platform.as_ref().unwrap();
                    let library_platforms = library.platforms
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>();

                    if library_platforms.iter().find(|n| **n == *configured_platform).is_none() {
                        bail!(
                            "Platforms mismatch: library '{}' has platforms [{}] in PIO, which do not contain the configured platform '{}'",
                            lib_name,
                            library_platforms.join(", "),
                            configured_platform);
                    }

                    let library_frameworks = library.frameworks
                        .iter()
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>();

                    if library_frameworks.iter().find(|f| self.frameworks.iter().find(|f2 | **f == f2.as_str()).is_some()).is_none() {
                        bail!(
                            "Frameworks mismatch: library '{}' has frameworks [{}] in PIO, which do not overlap with the configured frameworks [{}]",
                            lib_name,
                            library_frameworks.join(", "),
                            self.frameworks.join(", "));
                    }
                } else {
                    bail!("Library '{}' is not known to PIO", lib_name);
                }
            }
        }

        info!("Resolved libraries: [{}]", self.libraries.join(", "));

        Ok(())
    }

    fn resolve_project(&mut self) -> Result<()> {
        if self.project.is_none() {
            let out = env::var("OUT_DIR")?;
            self.project = Some(PathBuf::from(out).join("pio"));
        }

        let project = self.project.as_ref().unwrap();

        fs::create_dir_all(project)?;
        fs::create_dir_all(project.join("src"))?;

        {
            let mut file = fs::File::create(project.join("platformio.ini"))?;
            self.write_platformini_content(&mut file)?;

            let mut file = fs::File::create(project.join("src").join("main.c"))?;
            self.write_src_content(&mut file)?;

            let mut file = fs::File::create(project.join("script.py"))?;
            self.write_python_middleware_content(&mut file)?;

            if self.is_esp_idf_with_arduino() {
                // A workaround for the fact that ESP-IDF arduino does not build with ESP-IDF out of the box
                // Check this: https://github.com/platformio/platform-espressif32/tree/master/examples/espidf-arduino-blink
                // And this: https://github.com/platformio/platform-espressif32/issues/24

                let mut file = fs::File::create(project.join("sdkconfig.defaults"))?;
                self.write_sdkconfig_arduino_content(&mut file)?;

                info!("Generating sdkconfig.defaults file as a workaround for ESP-IDF + Arduino Espressif32 Core build issues");
            }
        }

        let mut cmd = self.pio.as_ref().unwrap().project(project);

        cmd.arg("init").status()?;

        info!("Resolved project: {:?}", self.project.as_ref().unwrap().as_os_str());

        Ok(())
    }

    fn resolve_framework_by_board(&mut self) -> Result<()> {
        let pio = self.pio.as_ref().unwrap();

        let board_id = self.board.as_ref().unwrap().as_str();

        let boards: Vec<Board> = pio.boards(None as Option<String>)?
            .into_iter()
            .filter(|b| b.id == board_id)
            .collect::<Vec<_>>();

        if boards.is_empty() {
            bail!("Configured board '{}' is not known to PIO", board_id);
        }

        if boards.len() > 1 {
            bail!(
                "Configured board '{}' matches multiple boards in PIO: [{}]",
                board_id,
                boards.iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(", "));
        }

        let board = &boards[0];

        let target_pf_res = Self::get_default_platform_and_frameworks();
        if let Ok(target_pf) = target_pf_res {
            if board.platform == target_pf.0 {
                bail!(
                    "Platforms mismatch: configured board '{}' has platform '{}' in PIO, which does not match platform '{}' derived from the build target '{}'",
                    board.id,
                    board.platform,
                    target_pf.0,
                    Self::get_target()?);
            }

            if target_pf.1.iter().find(|f| board.frameworks.iter().find(|f2| **f == f2.as_str()).is_none()).is_some() {
                bail!(
                    "Frameworks mismatch: configured board '{}' has frameworks [{}] in PIO, which do not contain the frameworks [{}] derived from the build target '{}'",
                    board.id,
                    board.frameworks.join(", "),
                    target_pf.1.join(", "),
                Self::get_target()?);
            }

            if self.platform.is_none() {
                self.platform = Some(target_pf.0.into());
            }

            if self.frameworks.is_empty() {
                self.frameworks.push(target_pf.1[0].into());
            }
        }

        if let Some(defined_platform) = self.platform.as_ref() {
            if *defined_platform != board.platform {
                bail!(
                    "Platforms mismatch: configured board '{}' has platform '{}' in PIO, which does not match the configured platform '{}'",
                    board.id,
                    board.platform,
                    defined_platform);
            }
        } else {
            self.platform = Some(board.platform.clone());
        }

        if !self.frameworks.is_empty() {
            if self.frameworks.iter().find(|f| board.frameworks.iter().find(|f2| f2.as_str() == f.as_str()).is_none()).is_some() {
                bail!(
                    "Frameworks mismatch: configured board '{}' has frameworks [{}] in PIO, which do not contain the configured frameworks [{}]",
                    board.id,
                    board.frameworks.join(", "),
                    self.frameworks.join(", "));
            }
        } else {
            self.frameworks.push(board.frameworks[0].clone());
        }

        Ok(())
    }

    fn resolve_framework_all(&mut self) -> Result<()> {
        let target_pf_res = Self::get_default_platform_and_frameworks();

        let target_pf = if self.platform.is_none() && self.frameworks.is_empty() {
            Some(target_pf_res?)
        } else {
            target_pf_res.ok()
        };

        if let Some(target_pf) = target_pf {
            if let Some(defined_platform) = self.platform.as_ref() {
                if defined_platform != target_pf.0 {
                    bail!(
                        "Platforms mismatch: configured platform '{}' does not match platform '{}', which was derived from the build target '{}'",
                        defined_platform,
                        target_pf.0,
                        Self::get_target()?);
                }
            } else {
                self.platform = Some(target_pf.0.into());
            }

            if !self.frameworks.is_empty() {
                if target_pf.1.iter().find(|f| self.frameworks.iter().find(|f2| f2.as_str() == **f).is_some()).is_none() {
                    bail!(
                        "Frameworks mismatch: configured frameworks [{}] are not contained in the frameworks [{}], which were derived from the build target '{}'",
                        self.frameworks.join(", "),
                        target_pf.1.join(", "),
                        Self::get_target()?);
                }
            } else {
                self.frameworks.push(target_pf.1[0].into());
            }
        }

        let pio = self.pio.as_ref().unwrap();

        let mut frameworks = pio.frameworks(None as Option<String>)?;

        if !self.frameworks.is_empty() {
            let not_found_frameworks = self.frameworks
                .iter()
                .filter(|f| frameworks.iter().find(|f2| f2.name == f.as_str()).is_none())
                .map(|s| s.as_str())
                .collect::<Vec<_>>();

            if !not_found_frameworks.is_empty() {
                bail!("Configured frameworks [{}] are not known to PIO", not_found_frameworks.join(", "));
            }

            frameworks = frameworks.into_iter()
                .filter(|f| self.frameworks.iter().find(|f2| f.name == f2.as_str()).is_some())
                .collect::<Vec<_>>();
        }

        if let Some(defined_platform) = self.platform.as_ref() {
            let frameworks_for_platform = frameworks
                .clone()
                .into_iter()
                .filter(|f| f.platforms.iter().find(|p| p.as_str() == defined_platform).is_some())
                .collect::<Vec<_>>();

            if frameworks_for_platform.is_empty() {
                if !self.frameworks.is_empty() {
                    bail!(
                        "Platforms mismatch: configured frameworks [{}] have platforms [{}] in PIO, which do not contain the configured platform '{}'",
                        self.frameworks.join(", "),
                        frameworks[0].platforms.join(", "),
                        defined_platform);
                } else {
                    bail!("Configurted platform '{}' is not known to PIO", defined_platform);
                }
            }

            frameworks = frameworks_for_platform;
        } else {
            let mut common_platforms: Option<Vec<String>> = None;

            for framework in &frameworks {
                if let Some(mut cp) = common_platforms {
                    cp = cp.into_iter()
                        .filter(|p| framework.platforms.iter().find(|p2| p.as_str() == p2.as_str()).is_some())
                        .collect::<Vec<_>>();

                    if cp.is_empty() {
                        bail!("Frameworks [{}] do not have a common platform in PIO", frameworks.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "));
                    } else {
                        common_platforms = Some(cp);
                    }
                } else {
                    common_platforms = Some(framework.platforms.clone());
                }
            }

            if common_platforms.as_ref().unwrap().len() > 1 {
                bail!(
                    "Frameworks [{}] have multiple common platforms in PIO: [{}]",
                    frameworks.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
                    common_platforms.unwrap().join(", "));
            } else {
                self.platform = Some(common_platforms.unwrap()[0].clone());
            }
        }

        if self.frameworks.is_empty() {
            self.frameworks = frameworks.into_iter().map(|f| f.name).collect::<Vec<_>>();
        }

        let boards = pio.boards(None as Option<String>)?
            .into_iter()
            .filter(|b|
                b.platform == *self.platform.as_ref().unwrap()
                && self.frameworks.iter().find(|f| b.frameworks.iter().find(|f2| f.as_str() == f2.as_str()).is_none()).is_none())
            .collect::<Vec<_>>();

        if boards.is_empty() {
            bail!(
                "Configured platform '{}' and frameworks [{}] do not have any board defined in PIO",
                self.platform.as_ref().unwrap(),
                self.frameworks.join(", "));
        } else {
            self.board = Some(boards[0].id.clone());
        }

        Ok(())
    }

    fn write_platformini_content<W: Write>(&self, writer: &mut W) -> Result<()> {
        write!(
            writer,
r#"
[env:default]
platform = {}{}
framework = {}
board = {}{}
build_type = {}
extra_scripts = script.py
"#,
            self.platform.as_ref().unwrap(),
            if self.is_esp_idf_with_arduino() {
                // A workaround for the fact that ESP-IDF arduino does not build with ESP-IDF >= 4.1
                info!("Using v4.0 of ESP-IDF Arduino core file as a workaround for ESP-IDF + Arduino Espressif32 Core build issues");

                "\nplatform_packages = framework-arduinoespressif32 @ https://github.com/espressif/arduino-esp32.git#idf-release/v4.0"
            } else {
                ""
            },
            self.frameworks.join(", "),
            self.board.as_ref().unwrap(),
            if self.libraries.is_empty() && self.unchecked_libraries.is_empty() {
                "".into()
            } else {
                format!(
                    "\nlib_deps = {}",
                    self.libraries.iter().map(|s| s.as_str())
                        .chain(self.unchecked_libraries.iter().map(|s| s.as_str()))
                        .collect::<Vec<_>>().join(", "))
            },
            env::var("PROFILE").ok().map_or("debug", |p| if p == "release" {"release"} else {"debug"}))?;

        Ok(())
    }

    fn write_src_content<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write("void app_main() {} void setup() {} void loop() {}".as_bytes())?;

        Ok(())
    }

    fn write_python_middleware_content<W: Write>(&self, writer: &mut W) -> Result<()> {
        write!(
            writer,
r#"
Import("env");
print(env.subst("$LINKFLAGS"))
print("=============")
print(env.subst("$_LIBDIRFLAGS"))
print("=============")
print(env.subst("$_LIBFLAGS"))
print("=============")
print(env.subst("$LINKCOM"))
"#)?;

        Ok(())
    }

    fn write_sdkconfig_arduino_content<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write(r#"
# Override some defaults to enable Arduino framework
CONFIG_ENABLE_ARDUINO_DEPENDS=y
CONFIG_AUTOSTART_ARDUINO=y
CONFIG_ARDUINO_RUN_CORE1=y
CONFIG_ARDUINO_RUNNING_CORE=1
CONFIG_ARDUINO_EVENT_RUN_CORE1=y
CONFIG_ARDUINO_EVENT_RUNNING_CORE=1
CONFIG_ARDUINO_UDP_RUN_CORE1=y
CONFIG_ARDUINO_UDP_RUNNING_CORE=1
CONFIG_DISABLE_HAL_LOCKS=y
CONFIG_ARDUHAL_LOG_DEFAULT_LEVEL_ERROR=y
CONFIG_ARDUHAL_LOG_DEFAULT_LEVEL=1
CONFIG_ARDUHAL_PARTITION_SCHEME_DEFAULT=y
CONFIG_ARDUHAL_PARTITION_SCHEME="default"
CONFIG_AUTOCONNECT_WIFI=y
CONFIG_ARDUINO_SELECTIVE_WiFi=y
CONFIG_MBEDTLS_PSK_MODES=y
CONFIG_MBEDTLS_KEY_EXCHANGE_PSK=y
"#.as_bytes())?;

        Ok(())
    }

    fn is_esp_idf_with_arduino(&self) -> bool {
        self.platform.as_ref().unwrap() == "espressif32"
            && self.frameworks.iter().find(|f| f.as_str() == "arduino").is_some()
            && self.frameworks.iter().find(|f| f.as_str() == "espidf").is_some()
    }

    fn get_default_platform_and_frameworks() -> Result<(&'static str, Vec<&'static str>)> {
        let target = Self::get_target()?;

        Ok(match target.as_str() {
            "esp32-xtensa-none" => ("espressif32", vec!["espidf", "arduino"]),
            "esp8266-xtensa-none" => ("esp8266", vec!["espidf", "ardino"]),
            _ => bail!("Cannot derive default PIO platform and framework for target {}", target),
        })
    }

    fn get_target() -> Result<String> {
        Ok(env::var("TARGET")?)
    }
}
