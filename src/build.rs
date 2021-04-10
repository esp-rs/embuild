use std::{collections::{HashMap, HashSet}, env, ffi::OsStr, fs, io::Write, path::{Path, PathBuf}, process::Command};

use anyhow::*;
use bindgen::EnumVariation;
use log::{info, trace};

use crate::{Board, Library, Pio, PioInstaller};

const ENVIRONMENT_NAME: &str = "default";

struct TargetConf {
    platform: &'static str,
    mcu: &'static str,
    frameworks: Vec<&'static str>,
}

#[derive(Clone, Debug)]
pub enum CopyFiles {
    Main(Files),
    Library(String, Files),
}

#[derive(Default, Clone, Debug)]
pub struct Files {
    pub files: Vec<PathBuf>,
    pub dest_dir: PathBuf,
    pub symlink: bool,
}

// TODO - INPUT ENV VARS:
// 1) Target configuration (optional?)
// 2) Path containing the linker (optional?) - not used, just assumed
// 3) (Header) files necessary to put as a reference in the PIO project (e.g. sdkconfig.h) (optional)

#[derive(Default, Clone, Debug)]
pub struct Builder {
    pio: Option<Pio>,
    platform: Option<String>,
    mcu: Option<String>,
    frameworks: Vec<String>,
    libraries: Vec<String>,
    unchecked_libraries: Vec<String>,
    copy_files: Vec<CopyFiles>,
    build_flags: Vec<String>,
    board: Option<String>,
    bindgen: Vec<(PathBuf, PathBuf)>,
    link: Option<bool>,
    pio_project_dir: Option<PathBuf>,
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

    pub fn mcu(&mut self, mcu: impl Into<String>) -> &mut Self {
        self.mcu = Some(mcu.into());
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

    pub fn copy_files(&mut self, files: CopyFiles) -> &mut Self {
        self.copy_files.push(files);
        self
    }

    pub fn build_flags(&mut self, flags: impl Into<String>) -> &mut Self {
        self.build_flags.push(flags.into());
        self
    }

    pub fn unchecked_library(&mut self, library: impl Into<String>) -> &mut Self {
        self.unchecked_libraries.push(library.into());
        self
    }

    pub fn bindgen(&mut self, header_file: impl Into<PathBuf>, bindings_file: impl Into<PathBuf>) -> &mut Self {
        self.bindgen.push((header_file.into(), bindings_file.into()));
        self
    }

    pub fn link(&mut self) -> &mut Self {
        self.link = Some(true);
        self
    }

    pub fn nolink(&mut self) -> &mut Self {
        self.link = Some(false);
        self
    }

    pub fn pio_project_dir(&mut self, project: impl Into<PathBuf>) -> &mut Self {
        self.pio_project_dir = Some(project.into());
        self
    }

    pub fn run(&mut self) -> Result<()> {
        self.resolve()?;

        let mut cmd = self.pio.as_ref().unwrap()
            .project(self.pio_project_dir.as_ref().unwrap());

        cmd.arg("run").arg("-t").arg("printcons")/*.arg("-t").arg("nobuild")*/.status()?;

        let cons = serde_json::from_reader::<fs::File, HashMap<String, String>>(
            fs::File::open(self.pio_project_dir.as_ref().unwrap().join("__environment.json"))?)?;

        for (header_file, bindings_file) in &self.bindgen {
            info!(
                "Running Bindgen for header '{}' and bindings file '{}'",
                header_file.display(),
                bindings_file.display());

            Self::run_bindgen(
                &cons["LINK"],
                &cons["_CPPINCFLAGS"],
                header_file,
                bindings_file)?;
        }

        if self.link.unwrap_or(false) {
            info!("Gathering linker flags");

            eprintln!("{:?}", Self::collect_linkflags(
                [&cons["LINKFLAGS"], " ",
                &cons["_LIBFLAGS"], " ",
                &cons["_LIBDIRFLAGS"]].concat())?);
        }

        Ok(())
    }

    pub fn pio_rel_dir() -> PathBuf {
        PathBuf::from(".pio")
    }

    pub fn pio_libraries_rel_dir() -> PathBuf {
        Self::pio_rel_dir().join(ENVIRONMENT_NAME).join("libdeps")
    }

    pub fn pio_build_rel_dir() -> PathBuf {
        Self::pio_rel_dir().join("build").join(ENVIRONMENT_NAME)
    }

    pub fn pio_build_env_rel_dir() -> PathBuf {
        Self::pio_build_rel_dir().join(ENVIRONMENT_NAME)
    }

    pub fn resolve(&mut self) -> Result<()> {
        self.resolve_pio()?;
        self.resolve_platform()?;
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

    fn resolve_platform(&mut self) -> Result<()> {
        if self.board.is_some() {
            self.resolve_platform_by_board()?;
        } else {
            self.resolve_platform_all()?;
        }

        info!(
            "Resolved platform: '{}', MCU: '{}', frameworks: [{}]",
            self.platform.as_ref().unwrap(),
            self.mcu.as_ref().unwrap(),
            self.frameworks.join(", "));

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
        if self.pio_project_dir.is_none() {
            let out = env::var("OUT_DIR")?;
            self.pio_project_dir = Some(PathBuf::from(out).join("pio"));
        }

        let pio_project_dir = self.pio_project_dir.as_ref().unwrap();

        fs::create_dir_all(pio_project_dir)?;
        fs::create_dir_all(pio_project_dir.join("src"))?;

        {
            let mut file = fs::File::create(pio_project_dir.join("platformio.ini"))?;
            self.write_platformini_content(&mut file)?;

            let mut file = fs::File::create(pio_project_dir.join("src").join("main.c"))?;
            self.write_src_content(&mut file)?;

            let mut file = fs::File::create(pio_project_dir.join("script.py"))?;
            self.write_python_middleware_content(&mut file)?;

            if self.is_esp_idf_with_arduino() {
                // A workaround for the fact that ESP-IDF arduino does not build with ESP-IDF out of the box
                // Check this: https://github.com/platformio/platform-espressif32/tree/master/examples/espidf-arduino-blink
                // And this: https://github.com/platformio/platform-espressif32/issues/24

                let mut file = fs::File::create(pio_project_dir.join("sdkconfig.defaults"))?;
                self.write_sdkconfig_arduino_content(&mut file)?;

                info!("Generating sdkconfig.defaults file as a workaround for ESP-IDF + Arduino Espressif32 Core build issues");
            }
        }

        let mut cmd = self.pio.as_ref().unwrap().project(pio_project_dir);

        cmd.arg("init").status()?;

        for library in &self.libraries {
            let mut cmd = self.pio.as_ref().unwrap().project(pio_project_dir);

            cmd.arg("lib").arg("install").arg(library).arg("--no-save").status()?;
        }

        for files in &self.copy_files {
            let (files, dest, symlink) = match files {
                CopyFiles::Main(files) => (
                    &files.files,
                    pio_project_dir.join(&files.dest_dir),
                    files.symlink),
                CopyFiles::Library(lib, files) => (
                    &files.files,
                    pio_project_dir
                        .join(Self::pio_libraries_rel_dir())
                        .join(lib)
                        .join(&files.dest_dir),
                    files.symlink),
            };

            Self::copy_file(files, dest, symlink)?;
        }

        info!("Resolved project: {:?}", self.pio_project_dir.as_ref().unwrap().as_os_str());

        Ok(())
    }

    fn copy_file<P: AsRef<Path>>(files: &[P], dest: P, symlink: bool) -> Result<()> {
        for file in files {
            let dest_file = dest.as_ref().join(file.as_ref().file_name().unwrap());

            if symlink {
                #[cfg(target_family = "unix")]
                std::os::unix::fs::symlink(file, dest_file)?;

                #[cfg(target_family = "windows")]
                std::os::windows::fs::symlink_file(file, dest_file)?;
            } else {
                fs::copy(file, dest_file)?;
            }
        }

        Ok(())
    }

    fn resolve_platform_by_board(&mut self) -> Result<()> {
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

        let target_pmf_res = Self::get_default_platform_mcu_frameworks();
        if let Ok(target_pmf) = target_pmf_res {
            if board.platform != target_pmf.platform {
                bail!(
                    "Platforms mismatch: configured board '{}' has platform '{}' in PIO, which does not match platform '{}' derived from the build target '{}'",
                    board.id,
                    board.platform,
                    target_pmf.platform,
                    Self::get_target()?);
            }

            if board.mcu != target_pmf.mcu {
                bail!(
                    "MCUs mismatch: configured board '{}' has MCU '{}' in PIO, which does not match MCU '{}' derived from the build target '{}'",
                    board.id,
                    board.mcu,
                    target_pmf.mcu,
                    Self::get_target()?);
            }

            if target_pmf.frameworks.iter().find(|f| board.frameworks.iter().find(|f2| **f == f2.as_str()).is_none()).is_some() {
                bail!(
                    "Frameworks mismatch: configured board '{}' has frameworks [{}] in PIO, which do not contain the frameworks [{}] derived from the build target '{}'",
                    board.id,
                    board.frameworks.join(", "),
                    target_pmf.frameworks.join(", "),
                Self::get_target()?);
            }

            if self.platform.is_none() {
                info!(
                    "Configuring platform '{}' derived from the build target '{}'",
                    target_pmf.platform,
                    Self::get_target()?);

                self.platform = Some(target_pmf.platform.into());
            }

            if self.mcu.is_none() {
                info!(
                    "Configuring MCU '{}' derived from the build target '{}'",
                    target_pmf.mcu,
                    Self::get_target()?);

                self.mcu = Some(target_pmf.mcu.into());
            }

            if self.frameworks.is_empty() {
                info!(
                    "Configuring framework '{}' from the frameworks [{}] derived from the build target '{}'",
                    target_pmf.frameworks[0],
                    target_pmf.frameworks.join(", "),
                    Self::get_target()?);

                self.frameworks.push(target_pmf.frameworks[0].into());
            }
        }

        if let Some(configured_platform) = self.platform.as_ref() {
            if *configured_platform != board.platform {
                bail!(
                    "Platforms mismatch: configured board '{}' has platform '{}' in PIO, which does not match the configured platform '{}'",
                    board.id,
                    board.platform,
                    configured_platform);
            }
        } else {
            info!(
                "Configuring platform '{}' supported by the configured board '{}'",
                board.platform,
                board.id);

            self.platform = Some(board.platform.clone());
        }

        if let Some(configured_mcu) = self.mcu.as_ref() {
            if *configured_mcu != board.mcu {
                bail!(
                    "Platforms mismatch: configured board '{}' has MCU '{}' in PIO, which does not match the configured MCU '{}'",
                    board.id,
                    board.mcu,
                    configured_mcu);
            }
        } else {
            info!(
                "Configuring MCU '{}' supported by the configured board '{}'",
                board.mcu,
                board.id);

            self.mcu = Some(board.mcu.clone());
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
            info!(
                "Configuring framework '{}' from the frameworks [{}] supported by the configured board '{}'",
                board.frameworks[0],
                board.frameworks.join(", "),
                board.id);

            self.frameworks.push(board.frameworks[0].clone());
        }

        Ok(())
    }

    fn resolve_platform_all(&mut self) -> Result<()> {
        let target_pmf_res = Self::get_default_platform_mcu_frameworks();

        let target_pmf = if self.platform.is_none() && self.frameworks.is_empty() {
            Some(target_pmf_res?)
        } else {
            target_pmf_res.ok()
        };

        if let Some(target_pmf) = target_pmf {
            if let Some(configured_platform) = self.platform.as_ref() {
                if configured_platform != target_pmf.platform {
                    bail!(
                        "Platforms mismatch: configured platform '{}' does not match platform '{}', which was derived from the build target '{}'",
                        configured_platform,
                        target_pmf.platform,
                        Self::get_target()?);
                }
            } else {
                info!(
                    "Configuring platform '{}' derived from the build target '{}'",
                    target_pmf.platform,
                    Self::get_target()?);

                self.platform = Some(target_pmf.platform.into());
            }

            if let Some(configured_mcu) = self.mcu.as_ref() {
                if configured_mcu != target_pmf.mcu {
                    bail!(
                        "MCUs mismatch: configured MCU '{}' does not match MCU '{}', which was derived from the build target '{}'",
                        configured_mcu,
                        target_pmf.mcu,
                        Self::get_target()?);
                }
            } else {
                info!(
                    "Configuring MCU '{}' derived from the build target '{}'",
                    target_pmf.mcu,
                    Self::get_target()?);

                self.mcu = Some(target_pmf.mcu.into());
            }

            if !self.frameworks.is_empty() {
                if target_pmf.frameworks.iter().find(|f| self.frameworks.iter().find(|f2| f2.as_str() == **f).is_some()).is_none() {
                    bail!(
                        "Frameworks mismatch: configured frameworks [{}] are not contained in the frameworks [{}], which were derived from the build target '{}'",
                        self.frameworks.join(", "),
                        target_pmf.frameworks.join(", "),
                        Self::get_target()?);
                }
            } else {
                info!(
                    "Configuring framework '{}' from the frameworks [{}] derived from the build target '{}'",
                    target_pmf.frameworks[0],
                    target_pmf.frameworks.join(", "),
                    Self::get_target()?);

                self.frameworks.push(target_pmf.frameworks[0].into());
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
                bail!("(Some of) the configured frameworks [{}] are not known to PIO", not_found_frameworks.join(", "));
            }
        }

        if let Some(configured_platform) = self.platform.as_ref() {
            let frameworks_for_platform = frameworks
                .clone()
                .into_iter()
                .filter(|f| f.platforms.iter().find(|p| p.as_str() == configured_platform).is_some())
                .collect::<Vec<_>>();

            if frameworks_for_platform.is_empty() {
                bail!("Configured platform '{}' is not known to PIO", configured_platform);
            }

            frameworks = frameworks_for_platform;

            if !self.frameworks.is_empty() {
                let not_found_frameworks = self.frameworks
                    .iter()
                    .filter(|f| frameworks.iter().find(|f2| f2.name == f.as_str()).is_none())
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>();

                if !not_found_frameworks.is_empty() {
                    bail!(
                        "(Some of) the configured frameworks [{}] are not supported by the configured platform '{}'",
                        not_found_frameworks.join(", "),
                        configured_platform);
                }
            } else {
                info!(
                    "Configuring framework '{}' from the frameworks [{}] matching the configured platform '{}'",
                    frameworks[0].name,
                    frameworks.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "),
                    configured_platform);

                self.frameworks.push(frameworks[0].name.clone());
            }
        } else {
            let platforms = frameworks.into_iter()
                .filter(|f| self.frameworks.iter().find(|f2| f.name == f2.as_str()).is_some())
                .map(|f| f.platforms)
                .reduce(|s1, s2|
                    s1.into_iter().collect::<HashSet<_>>()
                        .intersection(&s2.into_iter().collect::<HashSet<_>>())
                        .map(|s| s.clone())
                        .collect::<Vec<_>>())
                .unwrap_or(Vec::new());

            if platforms.is_empty() {
                bail!("Cannot select a platform: configured frameworks [{}] do not have a common platform", self.frameworks.join(", "));
            }

            if platforms.len() > 1 {
                bail!(
                    "Cannot select a platform: configured frameworks [{}] have multiple common platforms: [{}]",
                    self.frameworks.join(", "),
                    platforms.join(", "));
            }

            info!(
                "Configuring platform '{}' as the only common one of the configured frameworks [{}]",
                platforms[0],
                self.frameworks.join(", "));

            self.platform = Some(platforms[0].clone());
        }

        let mut boards = pio.boards(None as Option<String>)?
            .into_iter()
            .filter(|b|
                b.platform == *self.platform.as_ref().unwrap()
                && self.frameworks.iter().find(|f| b.frameworks.iter().find(|f2| f.as_str() == f2.as_str()).is_none()).is_none())
            .collect::<Vec<_>>();

        trace!(
            "Boards supporting configured platform '{}' and configured frameworks [{}]: [{}]",
            self.platform.as_ref().unwrap(),
            self.frameworks.join(", "),
            boards.iter().map(|b| b.id.as_str()).collect::<Vec<_>>().join(", "));

        if boards.is_empty() {
            bail!(
                "Configured platform '{}' and frameworks [{}] do not have any matching board defined in PIO",
                self.platform.as_ref().unwrap(),
                self.frameworks.join(", "));
        } else {
            if self.mcu.is_some() {
                boards = boards
                    .into_iter()
                    .filter(|b| b.mcu == *self.mcu.as_ref().unwrap())
                    .collect::<Vec<_>>();

                if boards.is_empty() {
                    bail!(
                        "Configured platform '{}', MCU '{}' and frameworks [{}] do not have any matching board defined in PIO",
                        self.platform.as_ref().unwrap(),
                        self.mcu.as_ref().unwrap(),
                        self.frameworks.join(", "));
                }
            } else {
                let mcus = boards.iter()
                    .map(|b| b.mcu.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();

                if mcus.len() > 1 {
                    bail!(
                        "Configured platform '{}' and frameworks [{}] match multiple MCUs in PIO: [{}]",
                        self.platform.as_ref().unwrap(),
                        self.frameworks.join(", "),
                        mcus.join(", "));
                } else {
                    info!(
                        "Configuring MCU '{}' which supports configured platform '{}' and configured frameworks [{}]",
                        mcus[0],
                        self.platform.as_ref().unwrap(),
                        self.frameworks.join(", "));

                    self.mcu = Some(mcus[0].clone());
                }
            }

            info!(
                "Configuring board '{}' which supports configured platform '{}', MCU '{}' and configured frameworks [{}]",
                boards[0].id,
                self.platform.as_ref().unwrap(),
                self.mcu.as_ref().unwrap(),
                self.frameworks.join(", "));

            self.board = Some(boards[0].id.clone());
        }

        Ok(())
    }

    fn write_platformini_content<W: Write>(&self, writer: &mut W) -> Result<()> {
        let platform_packages_addon = if self.is_esp_idf_with_arduino() {
            // A workaround for the fact that ESP-IDF arduino does not build with ESP-IDF >= 4.1
            info!("Using v4.0 of ESP-IDF Arduino core file as a workaround for ESP-IDF + Arduino Espressif32 Core build issues");

            "\nplatform_packages = framework-arduinoespressif32 @ https://github.com/espressif/arduino-esp32.git#idf-release/v4.0"
        } else {
            ""
        };

        let libraries_addon = if self.libraries.is_empty() && self.unchecked_libraries.is_empty() {
            "".into()
        } else {
            format!(
                "\nlib_deps = {}",
                self.libraries.iter().map(|s| s.as_str())
                    .chain(self.unchecked_libraries.iter().map(|s| s.as_str()))
                    .collect::<Vec<_>>().join(", "))
        };

        let build_flags_addon = if self.build_flags.is_empty() {
            "".into()
        } else {
            format!("\nbuild_flags = {}", self.build_flags.join(" "))
        };

        write!(
            writer,
r#"
[env:{}]
check_tool = clangtidy
platform = {}
framework = {}
board = {}
build_type = {}
extra_scripts = script.py
{}{}{}
"#,
            ENVIRONMENT_NAME,
            self.platform.as_ref().unwrap(),
            self.frameworks.join(", "),
            self.board.as_ref().unwrap(),
            env::var("PROFILE").ok().map_or("debug", |p| if p == "release" {"release"} else {"debug"}),
            platform_packages_addon,
            libraries_addon,
            build_flags_addon)?;

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
import json

Import("env");

def print_cons(*args, **kwargs):
    with open("__environment.json", "w") as f:
        print(
            json.dumps({{
                "LINKFLAGS": env.subst("$LINKFLAGS"),
                "_LIBDIRFLAGS": env.subst("$_LIBDIRFLAGS"),
                "_LIBFLAGS": env.subst("$_LIBFLAGS"),
                "LINK": env.subst("$LINK"),
                "CPPFLAGS": env.subst("$CPPFLAGS"),
                "CPPPATH": env.subst("$CPPPATH"),
                "_CPPINCFLAGS": env.subst("$_CPPINCFLAGS"),
                "ENV": json.dumps(env.subst("$ENV"))
            }}),
            file = f
        )

env.AddCustomTarget(
    name="printcons",
    dependencies="$BUILD_DIR/${{PROGNAME}}.elf",
    actions=print_cons,
    title="Prints various construction variables",
    description=None,
    always_build=True
)
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

    fn run_bindgen(linker: impl AsRef<str>, includes: impl AsRef<str>, header_file: &Path, bindings_file: &Path) -> Result<()> {
        Ok(Self::create_bindgen_bindings(linker, includes, header_file)?
            .write_to_file(bindings_file)?)
    }

    fn create_bindgen_bindings(linker: impl AsRef<str>, includes: impl AsRef<str>, header_file: &Path) -> Result<bindgen::Bindings> {
        Self::create_bindgen_builder(linker, includes, header_file)?
            .generate()
            .map_err(|_| anyhow!("Failed to generate bindings"))
    }

    fn create_bindgen_builder(linker: impl AsRef<str>, includes: impl AsRef<str>, header_file: &Path) -> Result<bindgen::Builder> {
        let linker = if linker.as_ref().ends_with("-gcc") {
            // Workaround: the (-)-print-sysroot option does not output anything when invoked on the *-gcc executable
            // Find and use the *-ld executable instead, where this option does work
            [&linker.as_ref()[0..linker.as_ref().len() - "-gcc".len()], "-ld"].concat()
        } else {
            linker.as_ref().to_owned()
        };

        let output = Command::new(OsStr::new(&linker))
            .arg("--print-sysroot")
            .output()?;

        let sysroot = PathBuf::from(OsStr::new(std::str::from_utf8(&output.stdout)?.trim()));

        Ok(bindgen::Builder::default()
            .use_core()
            .layout_tests(false)
            .ctypes_prefix("c_types")
            .default_enum_style(EnumVariation::Rust { non_exhaustive: false } )
            .header(format!("{}", header_file.display()))
            .clang_arg(format!("--sysroot={}", sysroot.display()))
            .clang_arg(format!("-I{}/include", sysroot.display()))
            .clang_arg("-Isrc")
            .clang_arg("-D__bindgen")
            .clang_args(&["-target", "xtensa"])
            .clang_args(&["-x", "c"])
            .clang_args(if let Some(includes) = shlex::split(includes.as_ref()) {
                    includes
                } else {
                    vec![includes.as_ref().to_owned()]
                }))
    }

    fn collect_linkflags(link_flags: impl AsRef<str>) -> Result<Vec<String>> {
        // TODO: Need to put a rerun-if-changed policy here

        let flags = if let Some(link_flags) = shlex::split(link_flags.as_ref()) {
            link_flags
        } else {
            vec![link_flags.as_ref().to_owned()]
        };

        // Note: need to pass -Zextra-link-arg to Cargo for any flags which do not start with -L or -l
        Ok(flags.into_iter()
            .map(|f| ["cargo:rustc-link-arg=", f.as_str()].concat())
            .collect::<Vec<_>>())
    }

    fn is_esp_idf_with_arduino(&self) -> bool {
        self.platform.as_ref().unwrap() == "espressif32"
            && self.frameworks.iter().find(|f| f.as_str() == "arduino").is_some()
            && self.frameworks.iter().find(|f| f.as_str() == "espidf").is_some()
    }

    fn get_default_platform_mcu_frameworks() -> Result<TargetConf> {
        let target = Self::get_target()?;

        Ok(match target.as_str() {
            "esp32-xtensa-none" => TargetConf {
                platform: "espressif32",
                mcu: "esp32",
                frameworks: vec!["espidf", "arduino", "simba", "pumbaa"],
            },
            "esp8266-xtensa-none" => TargetConf {
                platform: "espressif8266",
                mcu: "esp8266",
                frameworks: vec!["esp8266-rtos-sdk", "esp8266-nonos-sdk", "ardino", "simba"],
            },
            _ => bail!("Cannot derive default PIO platform, MCU and frameworks for target '{}'", target),
        })
    }

    fn get_target() -> Result<String> {
        Ok(env::var("TARGET")?)
    }
}
