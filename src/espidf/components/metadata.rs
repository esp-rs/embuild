//! Component metadata as read from `idf_component.yml` on the filesystem.
//! This is used when checking if a component is installed and matches the current version spec.
#![allow(unused)]

use anyhow::{Context, Result};
use semver::VersionReq;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Metadata {
    pub description: String,
    pub version: String,
    pub dependencies: BTreeMap<String, Dependency>,
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Dependency {
    pub version: String,
}

/// Returns the component metadata read from `path/idf_component.yml` if it exists.
pub fn get_component_metadata(component_path: &Path) -> Result<Option<Metadata>> {
    let meta_path = component_path.join("idf_component.yml");
    if meta_path.is_file() {
        let meta = std::fs::read_to_string(&meta_path)?;
        let meta = serde_yaml::from_str::<Metadata>(&meta).context(format!(
            "Error parsing metadata in file '{}'",
            meta_path.display()
        ))?;
        Ok(Some(meta))
    } else {
        Ok(None)
    }
}

/// Check if a component exists in `target_path` and matches the `version_req`.
pub fn component_exists_and_matches(version_req: &VersionReq, target_path: &Path) -> Result<bool> {
    if let Some(metadata) = get_component_metadata(target_path)? {
        let installed_version = semver::Version::parse(&metadata.version).context(format!(
            "Failed to parse version '{}' of component in '{}'",
            metadata.version,
            target_path.display()
        ))?;
        Ok(version_req.matches(&installed_version))
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_resource(name: &str) -> PathBuf {
        PathBuf::from(format!(
            "{}{}{}",
            env!("CARGO_MANIFEST_DIR"),
            "/tests/resources/espidf/components/",
            name
        ))
    }

    #[test]
    fn test_get_component_metadata() {
        assert_eq!(get_component_metadata(&test_resource("foo")).unwrap(), None);

        // Will search for `idf_component.yml`
        assert_eq!(
            get_component_metadata(&test_resource("")).unwrap(),
            Some(Metadata {
                description: "mDNS".to_string(),
                version: "1.1.0".to_string(),
                dependencies: BTreeMap::from([(
                    "idf".to_string(),
                    Dependency {
                        version: ">=5.0".to_string()
                    }
                )]),
            })
        );
    }

    #[test]
    fn test_version_checking() {
        let valid_path = test_resource("");
        // Version is 1.1.0, first check a valid match
        assert!(
            component_exists_and_matches(&VersionReq::parse(">=1.1.0").unwrap(), &valid_path)
                .unwrap()
        );

        // Check a non-matching version
        assert!(
            !component_exists_and_matches(&VersionReq::parse(">=1.2.0").unwrap(), &valid_path)
                .unwrap()
        );
    }
}
