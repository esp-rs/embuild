use std::convert::TryFrom;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use log::*;
use serde::{Deserialize, Serialize};

use super::Resolution;
use crate::cargo::CargoCmd;
use crate::utils::OsStrExt;
use crate::{build, cargo};

pub const OPTION_QUICK_DUMP: &str = "quick_dump";
pub const OPTION_TERMINATE_AFTER_DUMP: &str = "terminate_after_dump";

const VAR_BUILD_ACTIVE: &str = "CARGO_PIO_BUILD_ACTIVE";
const VAR_BUILD_RELEASE: &str = "CARGO_PIO_BUILD_RELEASE_BUILD";
//const VAR_BUILD_BINDGEN_RUN: &str = "CARGO_PIO_BUILD_BINDGEN_RUN";
const VAR_BUILD_PROJECT_DIR: &str = "CARGO_PIO_BUILD_PROJECT_DIR";
const VAR_BUILD_PATH: &str = "CARGO_PIO_BUILD_PATH";
const VAR_BUILD_INC_FLAGS: &str = "CARGO_PIO_BUILD_INC_FLAGS";
const VAR_BUILD_LIB_FLAGS: &str = "CARGO_PIO_BUILD_LIB_FLAGS";
const VAR_BUILD_LIB_DIR_FLAGS: &str = "CARGO_PIO_BUILD_LIB_DIR_FLAGS";
const VAR_BUILD_LIBS: &str = "CARGO_PIO_BUILD_LIBS";
const VAR_BUILD_LINK_FLAGS: &str = "CARGO_PIO_BUILD_LINK_FLAGS";
const VAR_BUILD_LINK: &str = "CARGO_PIO_BUILD_LINK";
const VAR_BUILD_LINKCOM: &str = "CARGO_PIO_BUILD_LINKCOM";
const VAR_BUILD_MCU: &str = "CARGO_PIO_BUILD_MCU";
const VAR_BUILD_BINDGEN_EXTRA_CLANG_ARGS: &str = "CARGO_PIO_BUILD_BINDGEN_EXTRA_CLANG_ARGS";
const VAR_BUILD_PIO_PLATFORM_DIR: &str = "CARGO_PIO_BUILD_PIO_PLATFORM_DIR";
const VAR_BUILD_PIO_FRAMEWORK_DIR: &str = "CARGO_PIO_BUILD_PIO_FRAMEWORK_DIR";

const PLATFORMIO_GIT_PY: &[u8] = include_bytes!("resources/platformio.git.py.resource");
const PLATFORMIO_PATCH_PY: &[u8] = include_bytes!("resources/platformio.patch.py.resource");
const PLATFORMIO_DUMP_PY: &[u8] = include_bytes!("resources/platformio.dump.py.resource");
const PLATFORMIO_CARGO_PY: &[u8] = include_bytes!("resources/platformio.cargo.py.resource");

const LIB_RS: &[u8] = include_bytes!("resources/lib.rs.resource");

const MAIN_C: &[u8] = include_bytes!("resources/main.c.resource");
const DUMMY_C: &[u8] = include_bytes!("resources/dummy.c.resource");

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct SconsVariables {
    pub project_dir: PathBuf,
    pub release_build: bool,

    pub path: String,
    pub incflags: String,
    pub libflags: String,
    pub libdirflags: String,
    pub libs: String,
    pub linkflags: String,
    pub link: String,
    pub linkcom: String,
    pub mcu: String,
    pub clangargs: Option<String>,

    pub pio_platform_dir: String,
    pub pio_framework_dir: String,
}

impl SconsVariables {
    pub fn from_piofirst() -> Option<Self> {
        if env::var(VAR_BUILD_ACTIVE).is_ok() {
            Some(Self {
                project_dir: PathBuf::from(env::var(VAR_BUILD_PROJECT_DIR).ok()?),
                release_build: env::var(VAR_BUILD_RELEASE).ok()?.to_lowercase() == "true",

                path: env::var(VAR_BUILD_PATH).ok()?,
                incflags: env::var(VAR_BUILD_INC_FLAGS).ok()?,
                libflags: env::var(VAR_BUILD_LIB_FLAGS).ok()?,
                libdirflags: env::var(VAR_BUILD_LIB_DIR_FLAGS).ok()?,
                libs: env::var(VAR_BUILD_LIBS).ok()?,
                linkflags: env::var(VAR_BUILD_LINK_FLAGS).ok()?,
                link: env::var(VAR_BUILD_LINK).ok()?,
                linkcom: env::var(VAR_BUILD_LINKCOM).ok()?,
                mcu: env::var(VAR_BUILD_MCU).ok()?,
                clangargs: env::var(VAR_BUILD_BINDGEN_EXTRA_CLANG_ARGS).ok(),

                pio_platform_dir: env::var(VAR_BUILD_PIO_PLATFORM_DIR).ok()?,
                pio_framework_dir: env::var(VAR_BUILD_PIO_FRAMEWORK_DIR).ok()?,
            })
        } else {
            None
        }
    }

    pub fn from_dump(project_path: impl AsRef<Path>) -> Result<Self> {
        Ok(serde_json::from_reader(fs::File::open(
            project_path.as_ref().join("__pio_scons_dump.json"),
        )?)?)
    }

    pub fn full_path(&self, executable: impl AsRef<str>) -> Result<PathBuf> {
        Ok(which::which_in(
            executable.as_ref(),
            Some(&self.path),
            env::current_dir()?,
        )?)
    }
}

pub struct Builder {
    project_dir: PathBuf,
    options: Vec<(String, String)>,
    git_repos_enabled: bool,
    git_repos: Vec<(String, PathBuf)>,
    files: Vec<(PathBuf, PathBuf)>,
    platform_packages: Vec<(String, PathBuf)>,
    platform_packages_patches_enabled: bool,
    platform_packages_patches: Vec<(PathBuf, PathBuf)>,
    cargo_cmd: Option<CargoCmd>,
    cargo_options: Vec<String>,
    scons_dump_enabled: bool,
    c_entry_points_enabled: bool,
}

impl Builder {
    pub fn new(project_dir: impl AsRef<Path>) -> Self {
        Self {
            project_dir: project_dir.as_ref().to_owned(),
            options: Vec::new(),
            git_repos_enabled: false,
            git_repos: Vec::new(),
            files: Vec::new(),
            platform_packages: Vec::new(),
            platform_packages_patches_enabled: false,
            platform_packages_patches: Vec::new(),
            cargo_cmd: None,
            cargo_options: Vec::new(),
            scons_dump_enabled: false,
            c_entry_points_enabled: false,
        }
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    pub fn option(&mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> &mut Self {
        self.options
            .push((name.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    pub fn options<S>(&mut self, options: impl Iterator<Item = (S, S)>) -> &mut Self
    where
        S: AsRef<str>,
    {
        for (name, value) in options {
            self.option(name, value);
        }

        self
    }

    pub fn cargo_option(&mut self, option: impl AsRef<str>) -> &mut Self {
        self.cargo_options.push(option.as_ref().to_owned());
        self
    }

    pub fn cargo_options<S>(&mut self, options: impl Iterator<Item = S>) -> &mut Self
    where
        S: AsRef<str>,
    {
        for option in options {
            self.cargo_option(option);
        }

        self
    }

    pub fn enable_git_repos(&mut self) -> &mut Self {
        self.git_repos_enabled = true;
        self
    }

    pub fn git_repo(&mut self, repo: impl AsRef<str>, location: impl AsRef<Path>) -> &mut Self {
        self.enable_git_repos();
        self.git_repos
            .push((repo.as_ref().to_owned(), location.as_ref().to_owned()));
        self
    }

    pub fn file(&mut self, source: impl AsRef<Path>, dest: impl AsRef<Path>) -> &mut Self {
        self.files
            .push((source.as_ref().to_owned(), dest.as_ref().to_owned()));
        self
    }

    pub fn files<S>(&mut self, files: impl Iterator<Item = (S, S)>) -> &mut Self
    where
        S: AsRef<Path>,
    {
        for (source, dest) in files {
            self.file(source, dest);
        }

        self
    }

    pub fn platform_package(
        &mut self,
        package: impl AsRef<str>,
        location: impl AsRef<Path>,
    ) -> &mut Self {
        self.platform_packages
            .push((package.as_ref().to_owned(), location.as_ref().to_owned()));
        self
    }

    pub fn enable_platform_packages_patches(&mut self) -> &mut Self {
        self.platform_packages_patches_enabled = true;
        self
    }

    pub fn platform_package_patch(
        &mut self,
        patch: impl AsRef<Path>,
        location: impl AsRef<Path>,
    ) -> &mut Self {
        self.enable_platform_packages_patches();
        self.platform_packages_patches
            .push((patch.as_ref().to_owned(), location.as_ref().to_owned()));
        self
    }

    pub fn enable_cargo(&mut self, cargo_cmd: CargoCmd) -> &mut Self {
        self.cargo_cmd = Some(cargo_cmd);
        self
    }

    pub fn enable_scons_dump(&mut self) -> &mut Self {
        self.scons_dump_enabled = true;
        self
    }

    pub fn enable_c_entry_points(&mut self) -> &mut Self {
        self.c_entry_points_enabled = true;
        self
    }

    pub fn generate(&self, resolution: &Resolution) -> Result<PathBuf> {
        let mut options = vec![
            ("board".into(), resolution.board.clone()),
            ("platform".into(), resolution.platform.clone()),
            ("framework".into(), resolution.frameworks.join(", ")),
        ];

        self.generate_with_options(resolution, &mut options)?;

        options.extend(self.options.iter().cloned());
        self.create_platformio_ini(&options)?;

        Ok(self.project_dir.clone())
    }

    pub fn update(&self) -> Result<PathBuf> {
        if self.cargo_cmd.is_some() {
            self.create_file("platformio.cargo.py", PLATFORMIO_CARGO_PY)?;
        } else if self.c_entry_points_enabled {
            self.create_file(PathBuf::from("src").join("main.c"), MAIN_C)?;
        }

        if self.git_repos_enabled {
            self.create_file("platformio.git.py", PLATFORMIO_GIT_PY)?;
        }

        if self.platform_packages_patches_enabled {
            self.create_file("platformio.patch.py", PLATFORMIO_PATCH_PY)?;
        }

        if self.scons_dump_enabled {
            self.create_file("platformio.dump.py", PLATFORMIO_DUMP_PY)?;
        }

        Ok(self.project_dir.clone())
    }

    fn generate_with_options(
        &self,
        resolution: &Resolution,
        options: &mut Vec<(String, String)>,
    ) -> Result<()> {
        let mut extra_scripts = Vec::new();

        if let Some(cargo_cmd) = self.cargo_cmd {
            let cargo_crate = cargo::Crate::new(self.project_dir.as_path());

            let rust_lib = match cargo_cmd {
                CargoCmd::New(build_std) | CargoCmd::Init(build_std) => {
                    cargo_crate.create(
                        matches!(cargo_cmd, CargoCmd::Init(_)),
                        ["--lib", "--vcs", "none"]
                            .into_iter()
                            .chain(self.cargo_options.iter().map(|s| &s[..])),
                    )?;

                    let rust_lib = cargo_crate.set_library_type(["staticlib"])?;
                    cargo_crate.create_config_toml(Some(resolution.target.clone()), build_std)?;

                    self.create_file(PathBuf::from("src").join("lib.rs"), LIB_RS)?;

                    rust_lib
                }
                _ => cargo_crate.check_staticlib()?,
            };

            self.create_file("platformio.cargo.py", PLATFORMIO_CARGO_PY)?;
            self.create_file(PathBuf::from("src").join("dummy.c"), DUMMY_C)?;

            options.push(("rust_lib".to_owned(), rust_lib));
            options.push(("rust_target".to_owned(), resolution.target.clone()));
        } else if self.c_entry_points_enabled {
            self.create_file(PathBuf::from("src").join("main.c"), MAIN_C)?;
        }

        self.copy_files()?;

        if self.git_repos_enabled {
            self.create_file("platformio.git.py", PLATFORMIO_GIT_PY)?;
            extra_scripts.push("pre:platformio.git.py");

            if let Some(option) = self.get_git_repos_option()? {
                options.push(option);
            }
        }

        if let Some(option) = self.get_platform_packages_option()? {
            options.push(option);
        }

        if self.platform_packages_patches_enabled {
            self.create_file("platformio.patch.py", PLATFORMIO_PATCH_PY)?;
            extra_scripts.push("pre:platformio.patch.py");

            if let Some(option) = self.get_platform_packages_patches_option()? {
                options.push(option);
            }
        }

        if self.cargo_cmd.is_some() {
            extra_scripts.push("platformio.cargo.py");
        }

        if self.scons_dump_enabled {
            self.create_file("platformio.dump.py", PLATFORMIO_DUMP_PY)?;
            extra_scripts.push("platformio.dump.py");
        }

        if !extra_scripts.is_empty() {
            options.insert(0, ("extra_scripts".to_owned(), extra_scripts.join(", ")));
        }

        self.update_gitignore()?;

        Ok(())
    }

    fn get_git_repos_option(&self) -> Result<Option<(String, String)>> {
        Ok(if !self.git_repos.is_empty() {
            Some((
                "git_repos".into(),
                format!(
                    "\n{}",
                    self.git_repos
                        .iter()
                        .map(|repo| format!("  {}@{}", repo.0, repo.1.display()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ))
        } else {
            None
        })
    }

    fn get_platform_packages_option(&self) -> Result<Option<(String, String)>> {
        Ok(if !self.platform_packages.is_empty() {
            Some((
                "platform_packages".into(),
                format!(
                    "\n{}",
                    self.platform_packages
                        .iter()
                        .map(|package| format!("  {}@{}", package.0, package.1.display()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ))
        } else {
            None
        })
    }

    fn get_platform_packages_patches_option(&self) -> Result<Option<(String, String)>> {
        let result = self
            .platform_packages_patches
            .iter()
            .map(|patch| {
                format!(
                    "  {}@{}",
                    patch.1.display(),
                    patch.0.file_name().unwrap().to_string_lossy()
                )
            })
            .collect::<Vec<String>>()
            .join("\n");

        Ok(if !result.is_empty() {
            Some(("patches".into(), format!("\n{}\n", result)))
        } else {
            None
        })
    }

    fn create_platformio_ini(&self, options: &[(impl AsRef<str>, impl AsRef<str>)]) -> Result<()> {
        let platformio_ini_path = self.project_dir.join("platformio.ini");

        debug!("Creating file {}", platformio_ini_path.display());

        fs::write(
            platformio_ini_path,
            format!(
                r#"
; PlatformIO Project Configuration File
;
; Please visit documentation for options and examples
; https://docs.platformio.org/page/projectconf.html
[platformio]
default_envs = debug

[env]
{}

[env:debug]
build_type = debug

[env:release]
build_type = release
"#,
                options
                    .iter()
                    .map(|(key, value)| format!("{} = {}", key.as_ref(), value.as_ref()))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        )?;

        Ok(())
    }

    fn update_gitignore(&self) -> Result<()> {
        // TODO: Only do this if not done already
        debug!("Adding \".pio\" and \"CMakeFiles\" directories to .gitignore");

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(self.project_dir.join(".gitignore"))?;

        writeln!(file, ".pio\nCMakeFiles/")?;

        Ok(())
    }

    fn copy_files(&self) -> Result<()> {
        for file_pair in &self.files {
            let dest_file = self.project_dir.join(&file_pair.1);

            debug!("Creating/updating {}", dest_file.display());

            fs::create_dir_all(dest_file.parent().unwrap())?;
            fs::copy(&file_pair.0, dest_file)?;
        }

        Ok(())
    }

    fn create_file(&self, path: impl AsRef<Path>, data: &[u8]) -> Result<()> {
        let dest_file = self.project_dir.join(path.as_ref());

        debug!("Creating/updating {}", dest_file.display());

        fs::create_dir_all(dest_file.parent().unwrap())?;
        fs::write(dest_file, data)?;

        Ok(())
    }
}

impl TryFrom<&SconsVariables> for build::CInclArgs {
    type Error = anyhow::Error;

    fn try_from(scons: &SconsVariables) -> Result<Self> {
        Ok(Self {
            args: scons.incflags.clone(),
        })
    }
}

impl TryFrom<&SconsVariables> for build::LinkArgsBuilder {
    type Error = anyhow::Error;

    fn try_from(scons: &SconsVariables) -> Result<Self> {
        let project_dir = scons.project_dir.try_to_str()?;

        let mut libdirflags = vec!["-L".to_owned() + project_dir];
        libdirflags.extend(
            scons
                .libdirflags
                .split_ascii_whitespace()
                .map(str::to_owned),
        );

        let libflags = scons
            .libflags
            .split_ascii_whitespace()
            .map(|arg| {
                // Hack: convert the relative paths that Pio generates to absolute ones
                if arg.starts_with(".pio/") {
                    format!("{}/{}", project_dir, arg)
                } else if arg.starts_with(".pio\\") {
                    format!("{}\\{}", project_dir, arg)
                } else {
                    arg.to_owned()
                }
            })
            .collect();

        Ok(Self {
            libflags,
            linkflags: scons
                .linkflags
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect(),
            libdirflags,
            linker: Some(scons.full_path(&scons.link)?),
            force_ldproxy: false,
            dedup_libs: true,
            ..Default::default()
        })
    }
}
