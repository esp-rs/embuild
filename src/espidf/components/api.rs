//! API client and model for the IDF Components API.
//! The API exists at https://api.components.espressif.com and the web UI for it is
//! at https://components.espressif.com.
#![allow(unused)]

use crate::espidf::components::api;
use anyhow::Result;
use log::warn;
use serde::Deserialize;
use std::path::PathBuf;

const API_BASE_URL: &str = "https://api.components.espressif.com";

pub struct Client {
    base_url: String,
}

impl Client {
    pub fn new() -> Self {
        Self {
            base_url: API_BASE_URL.to_string(),
        }
    }

    pub fn get_component(&self, namespace: &str, name: &str) -> Result<api::WithVersions> {
        let url = format!("{}/components/{namespace}/{name}", self.base_url);
        let component = ureq::get(&url).call()?.into_json::<api::WithVersions>()?;
        Ok(component)
    }
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct WithVersions {
    pub name: String,
    pub namespace: String,
    pub versions: Vec<Version>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Version {
    pub component_hash: Option<String>,
    pub version: String,
    pub license: Option<License>,
    pub dependencies: Vec<Dependency>,
    pub url: String,
    pub yanked_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct License {
    name: String,
    url: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Dependency {
    is_public: bool,
    namespace: Option<String>,
    name: Option<String>,
    source: Option<String>,
    spec: String,
}

pub fn find_best_match(component: &WithVersions, spec: &semver::VersionReq) -> Option<Version> {
    let matching_versions: Vec<&Version> = component
        .versions
        .iter()
        .filter(|v| v.yanked_at.is_none())
        .filter(|v| match semver::Version::parse(&v.version) {
            Ok(v) => spec.matches(&v),
            Err(_) => {
                eprintln!(
                    "Failed to parse version '{}' of component '{}'. Ignoring that version.",
                    v.version, component.name
                );
                false
            }
        })
        .collect();

    matching_versions
        .into_iter()
        .max_by_key(|v| semver::Version::parse(v.version.as_str()).unwrap())
        .map(|v| (*v).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_resource(name: &str) -> String {
        let path = format!(
            "{}{}{}",
            env!("CARGO_MANIFEST_DIR"),
            "/tests/resources/espidf/components/api/",
            name
        );
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    #[ignore]
    fn test_get_component() {
        let client = api::Client::new();
        let res = client.get_component("espressif", "mdns").unwrap();
        println!("{:#?}", res);
    }

    #[test]
    fn test_json_parsing() {
        let res =
            serde_json::from_str::<WithVersions>(&test_resource("component_result.json")).unwrap();
        println!("{:#?}", res);
    }

    #[test]
    fn test_version_matching() {
        let res =
            serde_json::from_str::<WithVersions>(&test_resource("component_result.json")).unwrap();
        let spec = semver::VersionReq::parse("1.0").unwrap();
        let selected_version = find_best_match(&res, &spec).unwrap();
        assert_eq!(selected_version.version, "1.0.9".to_string());
    }
}
