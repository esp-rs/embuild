pub mod project;

use std::collections::{HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::*;
use log::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tempfile::*;

const INSTALLER_URL: &str = "https://raw.githubusercontent.com/platformio/platformio-core-installer/master/get-platformio.py";
const INSTALLER_BLOB: &[u8] = include_bytes!("../resources/get-platformio.py.resource");

#[cfg(windows)]
const PYTHON: &str = "python"; // No 'python3.exe' on Windows

#[cfg(not(windows))]
const PYTHON: &str = "python3";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LogLevel {
    Quiet,
    Standard,
    Verbose,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Standard
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Platform {
    pub ownername: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub license: String,
    pub for_desktop: bool,
    pub frameworks: Vec<String>,
    pub packages: Vec<String>,
    pub versions: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Framework {
    pub name: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub homepage: String,
    pub platforms: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct LibrariesPage {
    pub page: u32,
    pub perpage: u32,
    pub total: u32,
    #[serde(default)]
    pub items: Vec<Library>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Library {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub updated: String,
    pub dllifetime: u64,
    pub dlmonth: u64,
    pub examplenums: u32,
    pub versionname: String,
    pub ownername: String,
    #[serde(default)]
    pub authornames: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub frameworks: Vec<LibraryFrameworkOrPlatformRef>,
    #[serde(default)]
    pub platforms: Vec<LibraryFrameworkOrPlatformRef>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct LibraryFrameworkOrPlatformRef {
    pub name: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub mcu: String,
    pub fcpu: u64,
    pub ram: u64,
    pub rom: u64,
    pub frameworks: Vec<String>,
    pub vendor: String,
    pub url: String,
    #[serde(default)]
    pub connectivity: Vec<String>,
    #[serde(default)]
    pub debug: BoardDebug,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct BoardDebug {
    #[serde(default)]
    pub tools: HashMap<String, HashMap<String, bool>>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Pio {
    pub is_develop_core: bool,
    pub platformio_exe: PathBuf,
    pub penv_dir: PathBuf,
    pub installer_version: String,
    pub python_version: String,
    pub core_version: String,
    pub system: String,
    pub python_exe: PathBuf,
    pub cache_dir: PathBuf,
    pub penv_bin_dir: PathBuf,
    pub core_dir: PathBuf,

    #[serde(default)]
    pub log_level: LogLevel,
}

impl Pio {
    pub fn install(
        pio_dir: Option<impl AsRef<Path>>,
        log_level: LogLevel,
        download: bool,
    ) -> Result<Self> {
        let mut pio_installer = if download {
            PioInstaller::new_download()?
        } else {
            PioInstaller::new()?
        };

        if log_level == LogLevel::Quiet {
            pio_installer.silent();
        }

        if let Some(pio_dir) = pio_dir {
            let pio_dir = pio_dir.as_ref();

            if !pio_dir.exists() {
                fs::create_dir(&pio_dir)?;
            }

            pio_installer.pio(&pio_dir);
        }

        pio_installer.update()
    }

    pub fn install_default() -> Result<Self> {
        Self::install(
            Option::<PathBuf>::None,
            LogLevel::Standard,
            false, /*download*/
        )
    }

    pub fn get_default() -> Result<Self> {
        Self::get(
            Option::<PathBuf>::None,
            LogLevel::Standard,
            false, /*download*/
        )
    }

    pub fn get(
        pio_dir: Option<impl AsRef<Path>>,
        log_level: LogLevel,
        download: bool,
    ) -> Result<Self> {
        let mut pio_installer = if download {
            PioInstaller::new_download()?
        } else {
            PioInstaller::new()?
        };

        if log_level == LogLevel::Quiet {
            pio_installer.silent();
        }

        if let Some(pio_dir) = pio_dir {
            pio_installer.pio(pio_dir.as_ref());
        }

        pio_installer.check().map(|mut pio| {
            pio.log_level = log_level;
            pio
        })
    }

    pub fn check(output: &Output) -> Result<()> {
        if !output.status.success() {
            bail!(
                "PIO returned status code {:?} and error stream {}",
                output.status.code(),
                String::from_utf8(output.stderr.clone())?
            );
        }

        Ok(())
    }

    pub fn cmd(&self) -> Command {
        let mut command = Command::new(&self.platformio_exe);

        command.env("PLATFORMIO_CORE_DIR", &self.core_dir);

        command
    }

    pub fn run_cmd(&self) -> Command {
        let mut cmd = self.cmd();

        cmd.arg("run");

        match self.log_level {
            LogLevel::Quiet => {
                cmd.arg("-s");
            }
            LogLevel::Verbose => {
                cmd.arg("-v");
            }
            _ => (),
        }

        cmd
    }

    pub fn build(&self, project_path: impl AsRef<Path>, release: bool) -> Result<()> {
        let mut cmd = self.run_cmd();

        cmd.arg("-d")
            .arg(project_path.as_ref())
            .arg("-e")
            .arg(if release { "release" } else { "debug" });

        self.exec(&mut cmd)
    }

    pub fn exec_with_args(&self, args: &[impl AsRef<OsStr>]) -> Result<()> {
        let mut cmd = self.cmd();

        self.exec(cmd.args(args))
    }

    pub fn run_with_args(&self, args: &[impl AsRef<OsStr>]) -> Result<()> {
        let mut cmd = self.run_cmd();

        self.exec(cmd.args(args))
    }

    pub fn exec(&self, cmd: &mut Command) -> Result<()> {
        debug!("Running PlatformIO command: {:?}", cmd);

        if self.log_level == LogLevel::Quiet {
            // Suppress PlatformIO's "Warning! Ignore unknown configuration option `...` in section [...]"
            // ... and the Download Manager verbosity... it is not suppressed by passing "-s" to pio run, unfortunately
            cmd.stderr(Stdio::null());
            cmd.stdout(Stdio::null());
        }

        cmd.status()?;

        Ok(())
    }

    pub fn json<T: DeserializeOwned>(cmd: &mut Command) -> Result<T> {
        cmd.arg("--json-output");
        debug!("Running PlatformIO command {:?}", cmd);

        let output = cmd.output()?;

        Self::check(&output)?;

        Ok(serde_json::from_slice::<T>(&output.stdout)?)
    }

    pub fn boards(&self, id: Option<impl AsRef<str>>) -> Result<Vec<Board>> {
        let mut cmd = self.cmd();

        cmd.arg("boards");

        if let Some(search_str) = id.as_ref() {
            cmd.arg(search_str.as_ref());
        }

        let result = Self::json::<Vec<Board>>(&mut cmd);

        if let Some(search_str) = id {
            Ok(result?
                .into_iter()
                .filter(|b| b.id == search_str.as_ref())
                .collect::<Vec<_>>())
        } else {
            result
        }
    }

    pub fn library(&self, name: Option<impl AsRef<str>>) -> Result<Library> {
        let mut cmd = self.cmd();

        cmd.arg("lib").arg("show");

        if let Some(name) = name {
            cmd.arg("--name").arg(name.as_ref());
        }

        Self::json::<Library>(&mut cmd)
    }

    pub fn libraries<S: AsRef<str>>(&self, names: &[S]) -> Result<Vec<Library>> {
        let mut res = Vec::<Library>::new();

        loop {
            let mut cmd = self.cmd();

            cmd.arg("lib").arg("search");

            for name in names {
                cmd.arg("--name").arg(name.as_ref());
            }

            let page = Self::json::<LibrariesPage>(&mut cmd)?;

            for library in page.items {
                res.push(library);
            }

            if page.page == page.total {
                break Ok(res);
            }
        }
    }

    pub fn platforms(&self, name: Option<impl AsRef<str>>) -> Result<Vec<Platform>> {
        let mut cmd = self.cmd();

        cmd.arg("platform").arg("search");

        if let Some(search_str) = name.as_ref() {
            cmd.arg(search_str.as_ref());
        }

        let result = Self::json::<Vec<Platform>>(&mut cmd);

        if let Some(search_str) = name {
            Ok(result?
                .into_iter()
                .filter(|p| p.name == search_str.as_ref())
                .collect::<Vec<_>>())
        } else {
            result
        }
    }

    pub fn frameworks(&self, name: Option<impl AsRef<str>>) -> Result<Vec<Framework>> {
        let mut cmd = self.cmd();

        cmd.arg("platform").arg("frameworks");

        if let Some(search_str) = name.as_ref() {
            cmd.arg(search_str.as_ref());
        }

        let result = Self::json::<Vec<Framework>>(&mut cmd);

        if let Some(search_str) = name {
            Ok(result?
                .into_iter()
                .filter(|f| f.name == search_str.as_ref())
                .collect::<Vec<_>>())
        } else {
            result
        }
    }
}

#[derive(Debug)]
pub struct PioInstaller {
    installer_location: PathBuf,
    installer_temp: Option<TempPath>,
    pio_location: Option<PathBuf>,
    silent: bool,
}

impl PioInstaller {
    pub fn new() -> Result<Self> {
        Self::create(false)
    }

    pub fn new_download() -> Result<Self> {
        Self::create(true)
    }

    pub fn new_location(installer_location: impl Into<PathBuf>) -> Result<Self> {
        Self::check_python()?;

        Ok(Self {
            installer_location: installer_location.into(),
            installer_temp: None,
            pio_location: None,
            silent: false,
        })
    }

    pub fn silent(&mut self) -> &mut Self {
        self.silent = true;

        self
    }

    fn create(download: bool) -> Result<Self> {
        Self::check_python()?;

        let mut file = NamedTempFile::new()?;
        if download {
            debug!("Downloading get-platformio.py from {}", INSTALLER_URL);

            let mut reader = ureq::get(INSTALLER_URL).call()?.into_reader();
            std::io::copy(&mut reader, &mut file)?;
        } else {
            debug!("Using built-in get-platformio.py");

            file.write(INSTALLER_BLOB)?;
        }

        let temp_path = file.into_temp_path();

        Ok(Self {
            installer_location: temp_path.to_path_buf(),
            installer_temp: Some(temp_path),
            pio_location: None,
            silent: false,
        })
    }

    fn check_python() -> Result<()> {
        let mut cmd = Command::new(PYTHON);

        cmd.arg("--version");

        debug!("Checking installed {} version {:?}", PYTHON, cmd);

        let output = match cmd.output() {
            Ok(output) => output,
            Err(_) => bail!(
                "Failed to locate a {} executable. Is {} installed and on your $PATH?",
                PYTHON,
                PYTHON
            ),
        };

        if !output.status.success() {
            bail!(
                "Failed to locate a {} executable. Is {} installed and on your $PATH?",
                PYTHON,
                PYTHON
            );
        }

        let version_str = std::str::from_utf8(&output.stdout)?;
        if !version_str.starts_with("Python ") {
            bail!("Unexpected version returned from the {} executable: '{}'. Expecting a version string starting with 'Python '", PYTHON, version_str);
        }

        let version_str = &version_str["Python ".len()..];

        let version = version_str
            .split(".")
            .map(|s| s.parse::<u32>().ok())
            .collect::<Vec<_>>();

        if version.len() < 2 || version[0].is_none() || version[1].is_none() {
            bail!("Unexpected version returned from the {} executable: '{}'. Expecting a version string of type '<number>.<number>[.remainder]'", PYTHON, version_str);
        }

        let major = version[0].unwrap();
        let minor = version[1].unwrap();
        if major < 3 || minor < 6 {
            bail!("{} executable is having version '{}' which is lower than 3.6; please upgrade your Python installation", PYTHON, version_str);
        }

        Ok(())
    }

    pub fn pio(&mut self, pio_location: impl Into<PathBuf>) -> &mut Self {
        let pio_location = pio_location.into();

        debug!("Using PlatformIO installation {}", pio_location.display());

        self.pio_location = Some(pio_location);
        self
    }

    pub fn update(&self) -> Result<Pio> {
        if let Ok(pio) = self.check() {
            info!("PlatformIO is up-to-date");

            Ok(pio)
        } else {
            info!("PlatformIO needs to be installed or updated");

            self.install()?;
            Ok(self.check()?)
        }
    }

    pub fn install(&self) -> Result<()> {
        let mut cmd = self.command();

        debug!("Running command {:?}", cmd);

        if self.silent {
            // Suppress PlatformIO's installer verbose output
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }

        cmd.status()?;

        Ok(())
    }

    pub fn check(&self) -> Result<Pio> {
        let (file, path) = NamedTempFile::new()?.into_parts();

        let mut cmd = self.command();

        cmd.arg("check").arg("core").arg("--dump-state").arg(&path);

        debug!("Running command {:?}", cmd);

        if self.silent {
            // Suppress PlatformIO's installer verbose output
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }

        cmd.status()?;

        Ok(serde_json::from_reader::<File, Pio>(file)?)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(PYTHON);

        if let Some(pio_location) = self.pio_location.as_ref() {
            command.env("PLATFORMIO_CORE_DIR", pio_location);
        }

        command.arg(&self.installer_location);

        command
    }
}

#[derive(Clone, Debug)]
pub struct Resolver {
    pio: Pio,
    params: ResolutionParams,
}

#[derive(Clone, Debug, Default)]
pub struct ResolutionParams {
    pub board: Option<String>,
    pub mcu: Option<String>,
    pub platform: Option<String>,
    pub frameworks: Vec<String>,
    pub target: Option<String>,
}

impl TryFrom<ResolutionParams> for Resolution {
    type Error = anyhow::Error;

    fn try_from(params: ResolutionParams) -> Result<Self, Self::Error> {
        if let Some(board) = params.board {
            if let Some(mcu) = params.mcu {
                if let Some(platform) = params.platform {
                    if !params.frameworks.is_empty() {
                        if let Some(target) = params.target {
                            return Ok(Self {
                                board,
                                mcu,
                                platform,
                                frameworks: params.frameworks.clone(),
                                target,
                            });
                        }
                    }
                }
            }
        }

        bail!("Error - should not get to here");
    }
}

pub struct TargetConf {
    platform: &'static str,
    mcu: &'static str,
    frameworks: Vec<&'static str>,
}

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    pub board: String,
    pub mcu: String,
    pub platform: String,
    pub frameworks: Vec<String>,
    pub target: String,
}

impl Resolver {
    pub fn new(pio: Pio) -> Self {
        Self {
            pio,
            params: Default::default(),
        }
    }

    pub fn params(mut self, params: ResolutionParams) -> Self {
        self.params = params;

        self
    }

    pub fn board(mut self, board: impl Into<String>) -> Self {
        self.params.board = Some(board.into());

        self
    }

    pub fn mcu(mut self, mcu: impl Into<String>) -> Self {
        self.params.mcu = Some(mcu.into());

        self
    }

    pub fn platform(mut self, platform: impl Into<String>) -> Self {
        self.params.platform = Some(platform.into());

        self
    }

    pub fn frameworks(mut self, frameworks: Vec<String>) -> Self {
        self.params.frameworks = frameworks;

        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.params.target = Some(target.into());

        self
    }

    pub fn resolve(&self, mandatory_target_resolution: bool) -> Result<Resolution> {
        debug!("Resolving {:?}", self);

        let resolution = if self.params.board.is_some() {
            self.resolve_platform_by_board(mandatory_target_resolution)?
        } else {
            self.resolve_platform_all(mandatory_target_resolution)?
        };

        info!(
            "Resolved platform: '{}', MCU: '{}', board: '{}', frameworks: [{}]",
            resolution.platform,
            resolution.mcu,
            resolution.board,
            resolution.frameworks.join(", ")
        );

        Ok(resolution)
    }

    fn resolve_platform_by_board(&self, mandatory_target_resolution: bool) -> Result<Resolution> {
        let mut params = self.params.clone();

        let board_id = params.board.as_ref().unwrap().as_str();

        let mut boards: Vec<Board> = self
            .pio
            .boards(None as Option<String>)?
            .into_iter()
            .filter(|b| b.id == board_id)
            .collect::<Vec<_>>();

        if boards.is_empty() {
            bail!("Configured board '{}' is not known to PIO", board_id);
        }

        let target_pmf = self.get_default_platform_mcu_frameworks();
        let target_pmf = if mandatory_target_resolution {
            Some(target_pmf?)
        } else {
            target_pmf.ok()
        };

        if boards.len() > 1 {
            let platform = if params.platform.is_some() {
                params.platform.clone()
            } else if target_pmf.is_some() {
                target_pmf.as_ref().map(|t| t.platform.to_owned())
            } else {
                None
            };

            if let Some(configured_platform) = platform {
                boards = boards
                    .into_iter()
                    .filter(|b| b.platform == configured_platform)
                    .collect::<Vec<_>>();

                if boards.is_empty() {
                    bail!("None of the boards in PIO matching the configured board '{}' supports the configured platform '{}'", board_id, configured_platform);
                } else if boards.len() > 1 {
                    bail!("Should not happen: multiple boards in PIO found matching the configured board '{}' and the configured platform '{}'", board_id, configured_platform);
                }
            } else {
                bail!("Configured board '{}' matches multiple boards in PIO: [{}]; please specify platform or target for proper resolution",
                    board_id,
                    boards
                        .iter()
                        .map(|b| b.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "))
            }
        }

        let board = &boards[0];

        if let Some(target_pmf) = target_pmf {
            let target = self.params.target.as_ref().unwrap();

            if board.platform != target_pmf.platform {
                bail!(
                    "Platforms mismatch: configured board '{}' has platform '{}' in PIO, which does not match platform '{}' derived from the build target '{}'",
                    board.id,
                    board.platform,
                    target_pmf.platform,
                    target);
            }

            if board.mcu != target_pmf.mcu {
                bail!(
                    "MCUs mismatch: configured board '{}' has MCU '{}' in PIO, which does not match MCU '{}' derived from the build target '{}'",
                    board.id,
                    board.mcu,
                    target_pmf.mcu,
                    target);
            }

            if target_pmf
                .frameworks
                .iter()
                .find(|f| {
                    board
                        .frameworks
                        .iter()
                        .find(|f2| **f == f2.as_str())
                        .is_none()
                })
                .is_some()
            {
                bail!(
                    "Frameworks mismatch: configured board '{}' has frameworks [{}] in PIO, which do not contain the frameworks [{}] derived from the build target '{}'",
                    board.id,
                    board.frameworks.join(", "),
                    target_pmf.frameworks.join(", "),
                    target);
            }

            if params.platform.is_none() {
                info!(
                    "Configuring platform '{}' derived from the build target '{}'",
                    target_pmf.platform,
                    self.params.target.as_ref().unwrap()
                );

                params.platform = Some(target_pmf.platform.into());
            }

            if params.mcu.is_none() {
                info!(
                    "Configuring MCU '{}' derived from the build target '{}'",
                    target_pmf.mcu, target
                );

                params.mcu = Some(target_pmf.mcu.into());
            }

            if params.frameworks.is_empty() {
                info!(
                    "Configuring framework '{}' from the frameworks [{}] derived from the build target '{}'",
                    target_pmf.frameworks[0],
                    target_pmf.frameworks.join(", "),
                    target);

                params.frameworks.push(target_pmf.frameworks[0].into());
            }
        }

        if let Some(configured_platform) = params.platform.as_ref() {
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
                board.platform, board.id
            );

            params.platform = Some(board.platform.clone());
        }

        if let Some(configured_mcu) = params.mcu.as_ref() {
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
                board.mcu, board.id
            );

            params.mcu = Some(board.mcu.clone());
        }

        if !params.frameworks.is_empty() {
            if params
                .frameworks
                .iter()
                .find(|f| {
                    board
                        .frameworks
                        .iter()
                        .find(|f2| f2.as_str() == f.as_str())
                        .is_none()
                })
                .is_some()
            {
                bail!(
                    "Frameworks mismatch: configured board '{}' has frameworks [{}] in PIO, which do not contain the configured frameworks [{}]",
                    board.id,
                    board.frameworks.join(", "),
                    params.frameworks.join(", "));
            }
        } else {
            info!(
                "Configuring framework '{}' from the frameworks [{}] supported by the configured board '{}'",
                board.frameworks[0],
                board.frameworks.join(", "),
                board.id);

            params.frameworks.push(board.frameworks[0].clone());
        }

        if params.target.is_none() {
            params.target = Some(Self::derive_target(params.mcu.as_ref().unwrap())?.to_owned());
        }

        params.try_into()
    }

    fn resolve_platform_all(&self, mandatory_target_resolution: bool) -> Result<Resolution> {
        let mut params = self.params.clone();

        let target_pmf = self.get_default_platform_mcu_frameworks();
        let target_pmf = if mandatory_target_resolution {
            Some(target_pmf?)
        } else {
            target_pmf.ok()
        };

        if let Some(target_pmf) = target_pmf {
            let target = self.params.target.as_ref().unwrap();

            if let Some(configured_platform) = params.platform.as_ref() {
                if configured_platform != target_pmf.platform {
                    bail!(
                        "Platforms mismatch: configured platform '{}' does not match platform '{}', which was derived from the build target '{}'",
                        configured_platform,
                        target_pmf.platform,
                        target);
                }
            } else {
                info!(
                    "Configuring platform '{}' derived from the build target '{}'",
                    target_pmf.platform, target
                );

                params.platform = Some(target_pmf.platform.into());
            }

            if let Some(configured_mcu) = params.mcu.as_ref() {
                if configured_mcu != target_pmf.mcu {
                    bail!(
                        "MCUs mismatch: configured MCU '{}' does not match MCU '{}', which was derived from the build target '{}'",
                        configured_mcu,
                        target_pmf.mcu,
                        target);
                }
            } else {
                info!(
                    "Configuring MCU '{}' derived from the build target '{}'",
                    target_pmf.mcu, target
                );

                params.mcu = Some(target_pmf.mcu.into());
            }

            if !params.frameworks.is_empty() {
                if target_pmf
                    .frameworks
                    .iter()
                    .find(|f| {
                        params
                            .frameworks
                            .iter()
                            .find(|f2| f2.as_str() == **f)
                            .is_some()
                    })
                    .is_none()
                {
                    bail!(
                        "Frameworks mismatch: configured frameworks [{}] are not contained in the frameworks [{}], which were derived from the build target '{}'",
                        params.frameworks.join(", "),
                        target_pmf.frameworks.join(", "),
                        target);
                }
            } else {
                info!(
                    "Configuring framework '{}' from the frameworks [{}] derived from the build target '{}'",
                    target_pmf.frameworks[0],
                    target_pmf.frameworks.join(", "),
                    target);

                params.frameworks.push(target_pmf.frameworks[0].into());
            }
        }

        let mut frameworks = self.pio.frameworks(None as Option<String>)?;

        if !params.frameworks.is_empty() {
            let not_found_frameworks = params
                .frameworks
                .iter()
                .filter(|f| frameworks.iter().find(|f2| f2.name == f.as_str()).is_none())
                .map(|s| s.as_str())
                .collect::<Vec<_>>();

            if !not_found_frameworks.is_empty() {
                bail!(
                    "(Some of) the configured frameworks [{}] are not known to PIO",
                    not_found_frameworks.join(", ")
                );
            }
        }

        if let Some(configured_platform) = params.platform.as_ref() {
            let frameworks_for_platform = frameworks
                .clone()
                .into_iter()
                .filter(|f| {
                    f.platforms
                        .iter()
                        .find(|p| p.as_str() == configured_platform)
                        .is_some()
                })
                .collect::<Vec<_>>();

            if frameworks_for_platform.is_empty() {
                bail!(
                    "Configured platform '{}' is not known to PIO",
                    configured_platform
                );
            }

            frameworks = frameworks_for_platform;

            if !params.frameworks.is_empty() {
                let not_found_frameworks = params
                    .frameworks
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

                params.frameworks.push(frameworks[0].name.clone());
            }
        } else {
            let platforms = frameworks
                .into_iter()
                .filter(|f| {
                    params
                        .frameworks
                        .iter()
                        .find(|f2| f.name == f2.as_str())
                        .is_some()
                })
                .map(|f| f.platforms)
                .fold(None, |a: Option<Vec<String>>, s2: Vec<String>| {
                    if let Some(s1) = a {
                        Some(
                            s1.into_iter()
                                .collect::<HashSet<_>>()
                                .intersection(&s2.into_iter().collect::<HashSet<_>>())
                                .map(|s| s.clone())
                                .collect::<Vec<_>>(),
                        )
                    } else {
                        Some(s2)
                    }
                })
                .unwrap_or(Vec::new());

            if platforms.is_empty() {
                bail!("Cannot select a platform: configured frameworks [{}] do not have a common platform", params.frameworks.join(", "));
            }

            if platforms.len() > 1 {
                bail!(
                    "Cannot select a platform: configured frameworks [{}] have multiple common platforms: [{}]",
                    params.frameworks.join(", "),
                    platforms.join(", "));
            }

            info!(
                "Configuring platform '{}' as the only common one of the configured frameworks [{}]",
                platforms[0],
                params.frameworks.join(", "));

            params.platform = Some(platforms[0].clone());
        }

        let mut boards = self
            .pio
            .boards(None as Option<String>)?
            .into_iter()
            .filter(|b| {
                b.platform == *params.platform.as_ref().unwrap()
                    && params
                        .frameworks
                        .iter()
                        .find(|f| {
                            b.frameworks
                                .iter()
                                .find(|f2| f.as_str() == f2.as_str())
                                .is_none()
                        })
                        .is_none()
            })
            .collect::<Vec<_>>();

        trace!(
            "Boards supporting configured platform '{}' and configured frameworks [{}]: [{}]",
            params.platform.as_ref().unwrap(),
            params.frameworks.join(", "),
            boards
                .iter()
                .map(|b| b.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        if boards.is_empty() {
            bail!(
                "Configured platform '{}' and frameworks [{}] do not have any matching board defined in PIO",
                params.platform.as_ref().unwrap(),
                params.frameworks.join(", "));
        } else {
            if params.mcu.is_some() {
                boards = boards
                    .into_iter()
                    .filter(|b| b.mcu == *params.mcu.as_ref().unwrap())
                    .collect::<Vec<_>>();

                if boards.is_empty() {
                    bail!(
                        "Configured platform '{}', MCU '{}' and frameworks [{}] do not have any matching board defined in PIO",
                        params.platform.as_ref().unwrap(),
                        params.mcu.as_ref().unwrap(),
                        params.frameworks.join(", "));
                }
            } else {
                let mcus = boards
                    .iter()
                    .map(|b| b.mcu.clone())
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();

                if mcus.len() > 1 {
                    bail!(
                        "Configured platform '{}' and frameworks [{}] match multiple MCUs in PIO: [{}]",
                        params.platform.as_ref().unwrap(),
                        params.frameworks.join(", "),
                        mcus.join(", "));
                } else {
                    info!(
                        "Configuring MCU '{}' which supports configured platform '{}' and configured frameworks [{}]",
                        mcus[0],
                        params.platform.as_ref().unwrap(),
                        params.frameworks.join(", "));

                    params.mcu = Some(mcus[0].clone());
                }
            }

            info!(
                "Configuring board '{}' which supports configured platform '{}', MCU '{}' and configured frameworks [{}]",
                boards[0].id,
                params.platform.as_ref().unwrap(),
                params.mcu.as_ref().unwrap(),
                params.frameworks.join(", "));

            params.board = Some(boards[0].id.clone());
        }

        if params.target.is_none() {
            params.target = Some(Self::derive_target(params.mcu.as_ref().unwrap())?.to_owned());
        }

        params.try_into()
    }

    fn get_default_platform_mcu_frameworks(&self) -> Result<TargetConf> {
        if let Some(ref target) = self.params.target {
            Self::derive_target_conf(target)
        } else {
            bail!("No target")
        }
    }

    pub fn derive_target_conf(target: impl AsRef<str>) -> Result<TargetConf> {
        Ok(match target.as_ref() {
            // TODO: Add more if possible
            "xtensa-esp32-none-elf" | "xtensa-esp32-espidf" => TargetConf {
                platform: "espressif32",
                mcu: "ESP32",
                frameworks: vec!["espidf", "arduino", "simba", "pumbaa"],
            },
            "xtensa-esp32s2-none-elf" | "xtensa-esp32s2-espidf" => TargetConf {
                platform: "espressif32",
                mcu: "ESP32S2",
                frameworks: vec!["espidf", "arduino", "simba", "pumbaa"],
            },
            "xtensa-esp32s3-none-elf" | "xtensa-esp32s3-espidf" => TargetConf {
                platform: "espressif32",
                mcu: "ESP32S3",
                frameworks: vec!["espidf", "arduino", "simba", "pumbaa"],
            },
            "riscv32imc-esp-espidf" | "riscv32imac-esp-espidf" => TargetConf {
                platform: "espressif32",
                mcu: "ESP32C3", // TODO: Once ESP32C6 hits the market, this will no longer be the only option
                frameworks: vec!["espidf", "arduino"],
            },
            "xtensa-esp8266-none-elf" => TargetConf {
                platform: "espressif8266",
                mcu: "ESP8266",
                frameworks: vec!["esp8266-rtos-sdk", "esp8266-nonos-sdk", "ardino", "simba"],
            },
            _ => bail!(
                "Cannot derive default PIO platform, MCU and frameworks for target '{}'",
                target.as_ref()
            ),
        })
    }

    pub fn derive_target(mcu: impl AsRef<str>) -> Result<&'static str> {
        let mcu = mcu.as_ref().to_lowercase();

        Ok(if mcu.starts_with("32mx") || mcu.starts_with("32mz") {
            // 32 bit PIC
            "mipsel-unknown-none"
        } else if mcu.starts_with("msp430") {
            // MSP-430
            "msp430-none-elf"
        } else if mcu.starts_with("at90") || mcu.starts_with("atmega") || mcu.starts_with("attiny")
        {
            // Microchip AVR
            "avr-unknown-gnu-atmega328"
        } else if mcu.starts_with("efm32") {
            // ARM Cortex-M4
            "thumbv7em-none-eabi"
        } else if mcu.starts_with("lpc") {
            // ARM Cortex-M0
            "thumbv6m-none-eabi"
        } else if mcu == "esp32" {
            // ESP32
            "xtensa-esp32-espidf"
        } else if mcu == "esp32s2" {
            // ESP32S2
            "xtensa-esp32s2-espidf"
        } else if mcu == "esp32s3" {
            // ESP32S3
            "xtensa-esp32s3-espidf"
        } else if mcu == "esp32c3" || mcu == "esp32c6" {
            // ESP32CX
            "riscv32imc-esp-espidf"
        } else if mcu == "esp8266" {
            // ESP8266
            "xtensa-esp8266-none-elf"
        } else if mcu.starts_with("stm32f7") || mcu.starts_with("stm32h7") {
            // ARM Cortex-M7F
            "thumbv7em-none-eabihf"
        } else if mcu.starts_with("gd32vf103") {
            // RISCV32IMAC
            "riscv32imac-unknown-none-elf"
        } else if mcu.starts_with("stm32f3")
            || mcu.starts_with("stm32f4")
            || mcu.starts_with("stm32g4")
            || mcu.starts_with("stm32l4")
            || mcu.starts_with("stm32l4+")
        {
            // ARM Cortex-M4F
            "thumbv7em-none-eabihf"
        } else if mcu.starts_with("stm32g0")
            || mcu.starts_with("stm32l0")
            || mcu.starts_with("stm32f0")
        {
            // ARM Cortex-M0/M0+
            "thumbv6m-none-eabi"
        } else if mcu.starts_with("nrf51") {
            // ARM Cortex-M0/M0+
            "thumbv6m-none-eabi"
        } else if mcu.starts_with("nrf52") {
            // ARM Cortex-M4F
            "thumbv7em-none-eabihf"
        } else {
            bail!(
                "Cannot derive Rust target triple for MCU {}. Specify one manually",
                mcu
            );
        })
    }
}
