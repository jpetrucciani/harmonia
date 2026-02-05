use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::{Version, VersionKind, VersionReq};
use crate::ecosystem::traits::EcosystemPlugin;
use crate::error::{HarmoniaError, Result};

pub struct NodePlugin;

impl NodePlugin {
    fn read_deps(map: &serde_json::Map<String, serde_json::Value>) -> Vec<Dependency> {
        map.iter()
            .filter_map(|(name, value)| value.as_str().map(|v| (name, v)))
            .map(|(name, value)| Dependency {
                name: name.to_string(),
                constraint: VersionReq::new(value),
                is_internal: false,
            })
            .collect()
    }
}

impl EcosystemPlugin for NodePlugin {
    fn id(&self) -> &'static str {
        "node"
    }

    fn file_patterns(&self) -> &'static [&'static str] {
        &["package.json"]
    }

    fn parse_version(&self, path: &Path, content: &str) -> Result<Option<Version>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("package.json") {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_str(content)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let version = value
            .get("version")
            .and_then(|v| v.as_str())
            .map(|v| Version::new(v, VersionKind::Semver));
        Ok(version)
    }

    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("package.json") {
            return Ok(Vec::new());
        }
        let value: serde_json::Value = serde_json::from_str(content)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let mut deps = Vec::new();
        for key in [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ] {
            if let Some(map) = value.get(key).and_then(|v| v.as_object()) {
                deps.extend(Self::read_deps(map));
            }
        }
        Ok(deps)
    }

    fn update_version(&self, path: &Path, content: &str, new_version: &Version) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("package.json") {
            return Ok(content.to_string());
        }
        let mut value: serde_json::Value = serde_json::from_str(content)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "version".to_string(),
                serde_json::Value::String(new_version.raw.clone()),
            );
        }
        serde_json::to_string_pretty(&value)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
    }

    fn update_dependency(
        &self,
        path: &Path,
        content: &str,
        dep: &str,
        constraint: &str,
    ) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("package.json") {
            return Ok(content.to_string());
        }
        let mut value: serde_json::Value = serde_json::from_str(content)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        for key in [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ] {
            if let Some(map) = value.get_mut(key).and_then(|v| v.as_object_mut()) {
                if map.contains_key(dep) {
                    map.insert(
                        dep.to_string(),
                        serde_json::Value::String(constraint.to_string()),
                    );
                    break;
                }
            }
        }
        serde_json::to_string_pretty(&value)
            .map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
    }

    fn default_test_command(&self) -> Option<&'static str> {
        Some("npm test")
    }

    fn default_lint_command(&self) -> Option<&'static str> {
        Some("npm run lint")
    }
}
