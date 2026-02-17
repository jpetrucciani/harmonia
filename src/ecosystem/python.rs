use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::{Version, VersionKind, VersionReq};
use crate::ecosystem::traits::EcosystemPlugin;
use crate::error::{HarmoniaError, Result};

pub struct PythonPlugin;

impl PythonPlugin {
    fn parse_pep508(req: &str) -> (String, Option<String>, Option<String>, Option<String>) {
        let (req_part, marker) = req.split_once(';').unwrap_or((req, ""));
        let marker = marker.trim();
        let marker = if marker.is_empty() {
            None
        } else {
            Some(marker.to_string())
        };
        let req_part = req_part.trim();
        let mut name_end = req_part.len();
        for (idx, ch) in req_part.char_indices() {
            if matches!(ch, '<' | '>' | '=' | '!' | '~' | '@' | ' ') {
                name_end = idx;
                break;
            }
        }
        let (name_part, rest) = req_part.split_at(name_end);
        let name_part = name_part.trim();
        let (name, extras) = match name_part.split_once('[') {
            Some((base, rest)) => {
                let extras = rest.trim_end_matches(']');
                (base.to_string(), Some(format!("[{}]", extras)))
            }
            None => (name_part.to_string(), None),
        };
        let constraint = rest.trim();
        let constraint = if constraint.is_empty() {
            None
        } else {
            Some(constraint.to_string())
        };
        (name, constraint, extras, marker)
    }

    fn rewrite_req(
        name: &str,
        constraint: &str,
        extras: Option<&str>,
        marker: Option<&str>,
    ) -> String {
        let mut out = String::new();
        out.push_str(name);
        if let Some(extras) = extras {
            out.push_str(extras);
        }
        if !constraint.is_empty() {
            out.push(' ');
            out.push_str(constraint.trim());
        }
        if let Some(marker) = marker {
            out.push_str("; ");
            out.push_str(marker.trim());
        }
        out
    }
}

impl EcosystemPlugin for PythonPlugin {
    fn id(&self) -> &'static str {
        "python"
    }

    fn file_patterns(&self) -> &'static [&'static str] {
        &["pyproject.toml"]
    }

    fn parse_version(&self, path: &Path, content: &str) -> Result<Option<Version>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("pyproject.toml") {
            return Ok(None);
        }
        let value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let version = value
            .get("project")
            .and_then(|project| project.get("version"))
            .and_then(|v| v.as_str())
            .map(|v| Version::new(v, VersionKind::Semver));
        Ok(version)
    }

    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("pyproject.toml") {
            return Ok(Vec::new());
        }
        let value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let deps = value
            .get("project")
            .and_then(|project| project.get("dependencies"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|val| val.as_str())
                    .map(|req| {
                        let (name, constraint, _, _) = Self::parse_pep508(req);
                        Dependency {
                            name,
                            constraint: VersionReq::new(constraint.unwrap_or_default()),
                            is_internal: false,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(deps)
    }

    fn update_version(&self, path: &Path, content: &str, new_version: &Version) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("pyproject.toml") {
            return Ok(content.to_string());
        }
        let mut value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        if let Some(project) = value
            .as_table_mut()
            .and_then(|table| table.get_mut("project"))
            .and_then(|project| project.as_table_mut())
        {
            project.insert(
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
        if path.file_name().and_then(|n| n.to_str()) != Some("pyproject.toml") {
            return Ok(content.to_string());
        }
        let mut value: toml::Value =
            toml::from_str(content).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))?;
        let deps = value
            .as_table_mut()
            .and_then(|table| table.get_mut("project"))
            .and_then(|project| project.as_table_mut())
            .and_then(|project| project.get_mut("dependencies"))
            .and_then(|deps| deps.as_array_mut());
        if let Some(deps) = deps {
            for entry in deps.iter_mut() {
                if let Some(req) = entry.as_str() {
                    let (name, _old_constraint, extras, marker) = Self::parse_pep508(req);
                    if name == dep {
                        let new_req = Self::rewrite_req(
                            &name,
                            constraint,
                            extras.as_deref(),
                            marker.as_deref(),
                        );
                        *entry = toml::Value::String(new_req);
                        break;
                    }
                }
            }
        }
        toml::to_string(&value).map_err(|err| HarmoniaError::Other(anyhow::Error::new(err)))
    }

    fn default_test_command(&self) -> Option<&'static str> {
        Some("pytest")
    }

    fn default_lint_command(&self) -> Option<&'static str> {
        Some("ruff check .")
    }
}

#[cfg(test)]
mod tests {
    use crate::ecosystem::python::PythonPlugin;
    use crate::ecosystem::traits::EcosystemPlugin;

    #[test]
    fn parses_and_updates_pyproject_dependencies() {
        let plugin = PythonPlugin;
        let path = std::path::Path::new("pyproject.toml");
        let content = r#"
[project]
name = "svc"
version = "1.0.0"
dependencies = [
  "core>=1.2,<2",
  "httpx[socks]>=0.25; python_version >= '3.11'",
]
"#;

        let deps = plugin
            .parse_dependencies(path, content)
            .expect("parse deps");
        assert!(deps.iter().any(|dep| dep.name == "core"));
        assert!(deps.iter().any(|dep| dep.name == "httpx"));

        let updated = plugin
            .update_dependency(path, content, "httpx", ">=0.30")
            .expect("update dep");
        assert!(updated.contains("httpx[socks] >=0.30; python_version >= '3.11'"));
    }
}
