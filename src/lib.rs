use std::{collections::HashMap, fs::File, io::{BufRead, BufReader, Write}, path::{Path, PathBuf}, process::{Command, Output}};

use anyhow::*;
use tempfile::*;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

const INSTALLER_URL: &str = "https://raw.githubusercontent.com/platformio/platformio-core-installer/master/get-platformio.py";

pub mod build;

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
pub struct BoardDebug { #[serde(default)] pub tools: HashMap<String, HashMap<String, bool>> }

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
}

impl Pio {
    pub fn cmd(&self) -> Command {
        let mut command = Command::new(&self.platformio_exe);

        command.env("PLATFORMIO_CORE_DIR", &self.core_dir);

        command
    }

    pub fn project(&self, project: &Path) -> Command {
        let mut command = self.cmd();

        command.current_dir(project);

        command
    }

    pub fn check(output: &Output) -> Result<()> {
        if !output.status.success() {
            bail!("PIO returned status code {:?} and error stream {}", output.status.code(), String::from_utf8(output.stderr.clone())?);
        }

        Ok(())
    }

    pub fn json<T: DeserializeOwned>(cmd: &mut Command) -> Result<T> {
        let output = cmd.arg("--json-output").output()?;

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
            Ok(result?.into_iter().filter(|b| b.id == search_str.as_ref()).collect::<Vec<_>>())
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
                break Ok(res)
            }
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
            Ok(result?.into_iter().filter(|f| f.name == search_str.as_ref()).collect::<Vec<_>>())
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
}

impl<T: Into<PathBuf>> From<T> for PioInstaller {
    fn from(path: T) -> Self {
        Self {
            installer_location: path.into(),
            installer_temp: None,
            pio_location: None,
        }
    }
}

impl PioInstaller {
    pub fn location(installer_location: impl Into<PathBuf>) -> Self {
        PioInstaller::from(installer_location)
    }

    pub fn new() -> Result<Self> {
        let mut file = NamedTempFile::new()?;

        let mut reader = BufReader::new(ureq::get(INSTALLER_URL)
            .call()?
            .into_reader());

        let writer = file.as_file_mut();

        loop {
            let buffer = reader.fill_buf()?;

            writer.write(buffer)?;

            let length = buffer.len();
            reader.consume(length);

            if length == 0 {
                break;
            }
        }

        let temp_path = file.into_temp_path();

        Ok(Self {
            installer_location: temp_path.to_path_buf(),
            installer_temp: Some(temp_path),
            pio_location: None,
        })
    }

    pub fn pio(&mut self, pio_location: impl Into<PathBuf>) -> &mut Self {
        self.pio_location = Some(pio_location.into());
        self
    }

    pub fn update(&self) -> Result<Pio> {
        if let Ok(pio) = self.check() {
            Ok(pio)
        } else {
            self.install()?;
            Ok(self.check()?)
        }
    }

    pub fn install(&self) -> Result<()> {
        self.command().status()?;

        Ok(())
    }

    pub fn check(&self) -> Result<Pio> {
        let (file, path) = NamedTempFile::new()?.into_parts();

        self.command()
            .arg("check")
            .arg("core")
            .arg("--dump-state")
            .arg(&path)
            .status()?;

        Ok(serde_json::from_reader::<File, Pio>(file)?)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("python");

        if let Some(pio_location) = self.pio_location.as_ref() {
            command.env("PLATFORMIO_CORE_DIR", pio_location);
        }

        command.arg(&self.installer_location);

        command
    }
}
