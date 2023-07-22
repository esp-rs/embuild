use std::path::PathBuf;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use log::info;
use tar::Archive;

pub mod comp;

const API_BASE_URL: &str = "https://api.components.espressif.com";

struct ApiClient {
    base_url: String,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            base_url: API_BASE_URL.to_string(),
        }
    }

    pub fn get_component(&self, namespace: &str, name: &str) -> Result<comp::WithVersions> {
        let url = format!("{}/components/{namespace}/{name}", self.base_url);
        let component = ureq::get(&url).call()?.into_json::<comp::WithVersions>()?;
        Ok(component)
    }
}

pub struct IdfComponentDep {
    pub namespace: String,
    pub name: String,
    pub version_req: semver::VersionReq,
}

impl IdfComponentDep {
    pub fn new(namespace: String, name: String, version_req: semver::VersionReq) -> Self {
        Self { namespace, name, version_req }
    }
}

pub struct IdfComponentManager {
    components_dir: PathBuf,
    components: Vec<IdfComponentDep>,
    api_client: ApiClient,
}

impl IdfComponentManager {
    pub fn new(components_dir: PathBuf) -> Self {
        Self { components_dir, components: vec![], api_client: ApiClient::new() }
    }

    pub fn with_component(mut self, name: &str, version_spec: &str) -> Result<Self> {
        let version_req = semver::VersionReq::parse(&version_spec)
            .context(format!("Error parsing version request for {}", name))?;

        // Parse namespace and name from component in format "namespace/name"
        match name.split("/").collect::<Vec<&str>>().as_slice() {
            [namespace, name] => {
                self.components.push(
                    IdfComponentDep::new(namespace.to_string(), name.to_string(), version_req)
                );
            }
            _ => return Err(anyhow::anyhow!("Invalid component name {}", name)),
        }
        Ok(self)
    }

    pub fn install(self) -> Result<Vec<PathBuf>> {
        let mut component_dirs = vec![];
        for component in &self.components {
            let target_path = &self.components_dir.join(format!("{}__{}", component.namespace, component.name));

            info!("Installing component {}:{}...", component.name, component.version_req);
            let dir = self.install_component(component, target_path)?;
            component_dirs.push(dir);
        }
        Ok(component_dirs)
    }

    fn install_component(&self, component: &IdfComponentDep, target_path: &PathBuf) -> Result<PathBuf> {
        if target_path.exists() {
            info!("Component {} matching version spec {} is already installed.", component.name, component.version_req);
        } else {
            let metadata = self.api_client.get_component(&component.namespace, &component.name)
                .context(format!("Failed to get component {} from API", component.name))?;

            let version = comp::find_best_match(&metadata, &component.version_req)
                .context(format!("No matching version found for component {} with version spec {}", component.name, component.version_req))?;
            info!("Downloading component {}:{} from {} to {}...", component.name, version.version, version.url, target_path.display());
            download_and_unpack(version.url.as_str(), &target_path)?;
        }

        Ok(target_path.clone())
    }
}

fn download_and_unpack(tarball_url: &str, target_path: &PathBuf) -> Result<()> {
    let response = ureq::get(tarball_url).call()?;
    let mut tar = Archive::new(GzDecoder::new(response.into_reader()));
    tar.unpack(target_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_get_component() {
        let client = ApiClient::new();
        let res = client.get_component("espressif", "mdns").unwrap();
        println!("{:#?}", res);
    }

    #[test]
    #[ignore]
    fn test_unpack() {
        let tmp_dir = tempdir::TempDir::new("managed_components").unwrap().into_path();

        let mgr = IdfComponentManager::new(tmp_dir)
            .with_component("espressif/mdns".into(), "1.1.0".into())
            .unwrap();

        let paths = mgr.install().unwrap();
        println!("Final component path: {}", paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
    }
}