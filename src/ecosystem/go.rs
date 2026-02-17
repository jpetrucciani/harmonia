use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::{Version, VersionReq};
use crate::ecosystem::traits::EcosystemPlugin;
use crate::error::Result;

pub struct GoPlugin;

impl GoPlugin {
    fn parse_require_line(line: &str) -> Option<(String, String)> {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            return None;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next()?.to_string();
        let version = parts.next()?.to_string();
        Some((name, version))
    }
}

impl EcosystemPlugin for GoPlugin {
    fn id(&self) -> &'static str {
        "go"
    }

    fn file_patterns(&self) -> &'static [&'static str] {
        &["go.mod"]
    }

    fn parse_version(&self, _path: &Path, _content: &str) -> Result<Option<Version>> {
        Ok(None)
    }

    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>> {
        if path.file_name().and_then(|n| n.to_str()) != Some("go.mod") {
            return Ok(Vec::new());
        }
        let mut deps = Vec::new();
        let mut in_block = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("require (") {
                in_block = true;
                continue;
            }
            if in_block && trimmed.starts_with(')') {
                in_block = false;
                continue;
            }
            if trimmed.starts_with("require ") {
                let rest = trimmed.trim_start_matches("require").trim();
                if let Some((name, version)) = Self::parse_require_line(rest) {
                    deps.push(Dependency {
                        name,
                        constraint: VersionReq::new(version),
                        is_internal: false,
                    });
                }
                continue;
            }
            if in_block {
                if let Some((name, version)) = Self::parse_require_line(trimmed) {
                    deps.push(Dependency {
                        name,
                        constraint: VersionReq::new(version),
                        is_internal: false,
                    });
                }
            }
        }
        Ok(deps)
    }

    fn update_version(
        &self,
        _path: &Path,
        content: &str,
        _new_version: &Version,
    ) -> Result<String> {
        Ok(content.to_string())
    }

    fn update_dependency(
        &self,
        path: &Path,
        content: &str,
        dep: &str,
        constraint: &str,
    ) -> Result<String> {
        if path.file_name().and_then(|n| n.to_str()) != Some("go.mod") {
            return Ok(content.to_string());
        }
        let mut out = Vec::new();
        let mut in_block = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("require (") {
                in_block = true;
                out.push(line.to_string());
                continue;
            }
            if in_block && trimmed.starts_with(')') {
                in_block = false;
                out.push(line.to_string());
                continue;
            }
            if trimmed.starts_with("require ") {
                let rest = trimmed.trim_start_matches("require").trim();
                if let Some((name, _)) = Self::parse_require_line(rest) {
                    if name == dep {
                        out.push(format!("require {dep} {constraint}"));
                        continue;
                    }
                }
            }
            if in_block {
                if let Some((name, _)) = Self::parse_require_line(trimmed) {
                    if name == dep {
                        let prefix = line.split_whitespace().next().unwrap_or("");
                        if prefix == dep {
                            out.push(format!("\t{dep} {constraint}"));
                        } else {
                            out.push(format!("{dep} {constraint}"));
                        }
                        continue;
                    }
                }
            }
            out.push(line.to_string());
        }
        Ok(out.join("\n"))
    }

    fn default_test_command(&self) -> Option<&'static str> {
        Some("go test ./...")
    }

    fn default_lint_command(&self) -> Option<&'static str> {
        Some("golangci-lint run")
    }
}

#[cfg(test)]
mod tests {
    use crate::ecosystem::go::GoPlugin;
    use crate::ecosystem::traits::EcosystemPlugin;

    #[test]
    fn parses_and_updates_go_mod_dependencies() {
        let plugin = GoPlugin;
        let path = std::path::Path::new("go.mod");
        let content = r#"
module example.com/svc

go 1.22

require (
    example.com/core v1.2.3
)
"#;

        let deps = plugin
            .parse_dependencies(path, content)
            .expect("parse deps");
        assert!(deps.iter().any(|dep| dep.name == "example.com/core"));

        let updated = plugin
            .update_dependency(path, content, "example.com/core", "v1.3.0")
            .expect("update dep");
        assert!(updated.contains("example.com/core v1.3.0"));
    }
}
