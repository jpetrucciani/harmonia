use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::Version;
use crate::ecosystem::traits::EcosystemPlugin;
use crate::error::Result;

pub struct CustomPlugin;

impl EcosystemPlugin for CustomPlugin {
    fn id(&self) -> &'static str {
        "custom"
    }

    fn file_patterns(&self) -> &'static [&'static str] {
        &[]
    }

    fn parse_version(&self, _path: &Path, _content: &str) -> Result<Option<Version>> {
        Ok(None)
    }

    fn parse_dependencies(&self, _path: &Path, _content: &str) -> Result<Vec<Dependency>> {
        Ok(Vec::new())
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
        _path: &Path,
        content: &str,
        _dep: &str,
        _constraint: &str,
    ) -> Result<String> {
        Ok(content.to_string())
    }

    fn default_test_command(&self) -> Option<&'static str> {
        None
    }

    fn default_lint_command(&self) -> Option<&'static str> {
        None
    }
}
