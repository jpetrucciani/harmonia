use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::{Version, VersionKind, VersionReq};
use crate::ecosystem::traits::EcosystemPlugin;
use crate::error::{HarmoniaError, Result};

pub struct RustPlugin;

impl RustPlugin {
    fn read_deps(table: &toml::value::Table) -> Vec<Dependency> {
        let mut deps = Vec::new();
        for (name, value) in table {
            let constraint = match value {
                toml::Value::String(s) => Some(s.clone()),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|v| v.to_string()),
                _ => None,
            };
            deps.push(Dependency {
                name: name.clone(),
                constraint: VersionReq::new(constraint.unwrap_or_default()),
                is_internal: false,
            });
        }
        deps
    }
}

impl EcosystemPlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn file_patterns(&self) -> &'static [&'static str] {
        &["Cargo.toml"]
    }

    fn parse_version(&self, path: &Path, content: &str) -> Result<Option<Version>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            return Ok(None);
        }
        let value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let version = value
            .get("package")
            .and_then(|pkg| pkg.get("version"))
            .and_then(|v| v.as_str())
            .map(|v| Version::new(v, VersionKind::Semver));
        Ok(version)
    }

    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            return Ok(Vec::new());
        }
        let value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let mut deps = Vec::new();
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(table) = value.get(section).and_then(|t| t.as_table()) {
                deps.extend(Self::read_deps(table));
            }
        }
        Ok(deps)
    }

    fn update_version(&self, path: &Path, content: &str, new_version: &Version) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            return Ok(content.to_string());
        }
        let mut value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if let Some(pkg) = value
            .as_table_mut()
            .and_then(|table| table.get_mut("package"))
            .and_then(|pkg| pkg.as_table_mut())
        {
            pkg.insert(
                "version".to_string(),
                toml::Value::String(new_version.raw.clone()),
            );
        }
        toml::to_string(&value).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
    }

    fn update_dependency(
        &self,
        path: &Path,
        content: &str,
        dep: &str,
        constraint: &str,
    ) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("Cargo.toml") {
            return Ok(content.to_string());
        }
        let mut value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(table) = value
                .as_table_mut()
                .and_then(|root| root.get_mut(section))
                .and_then(|t| t.as_table_mut())
            {
                if let Some(entry) = table.get_mut(dep) {
                    match entry {
                        toml::Value::String(s) => {
                            *s = constraint.to_string();
                        }
                        toml::Value::Table(t) => {
                            t.insert(
                                "version".to_string(),
                                toml::Value::String(constraint.to_string()),
                            );
                        }
                        _ => {}
                    }
                    break;
                }
            }
        }
        toml::to_string(&value).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
    }

    fn default_test_command(&self) -> Option<&'static str> {
        Some("cargo test")
    }

    fn default_lint_command(&self) -> Option<&'static str> {
        Some("cargo clippy")
    }
}

#[cfg(test)]
mod tests {
    use crate::core::version::{Version, VersionKind};
    use crate::ecosystem::rust::RustPlugin;
    use crate::ecosystem::traits::EcosystemPlugin;

    #[test]
    fn parses_and_updates_rust_manifest() {
        let plugin = RustPlugin;
        let path = std::path::Path::new("Cargo.toml");
        let content =
            "[package]\nname = \"svc\"\nversion = \"0.1.0\"\n\n[dependencies]\ncore = \"^0.1\"\n";

        let version = plugin
            .parse_version(path, content)
            .expect("parse version")
            .expect("version exists");
        assert_eq!(version.raw, "0.1.0");

        let deps = plugin
            .parse_dependencies(path, content)
            .expect("parse deps");
        assert!(deps.iter().any(|dep| dep.name == "core"));

        let updated = plugin
            .update_dependency(path, content, "core", "^0.2")
            .expect("update dep");
        assert!(updated.contains("core = \"^0.2\""));

        let updated_version = plugin
            .update_version(path, &updated, &Version::new("0.2.0", VersionKind::Semver))
            .expect("update version");
        assert!(updated_version.contains("version = \"0.2.0\""));
    }
}
