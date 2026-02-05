use std::path::Path;

use crate::core::repo::Dependency;
use crate::core::version::Version;
use crate::error::Result;

pub trait EcosystemPlugin: Send + Sync {
    fn id(&self) -> &'static str;
    fn file_patterns(&self) -> &'static [&'static str];
    fn parse_version(&self, path: &Path, content: &str) -> Result<Option<Version>>;
    fn parse_dependencies(&self, path: &Path, content: &str) -> Result<Vec<Dependency>>;
    fn update_version(&self, path: &Path, content: &str, new_version: &Version) -> Result<String>;
    fn update_dependency(
        &self,
        path: &Path,
        content: &str,
        dep: &str,
        constraint: &str,
    ) -> Result<String>;
    fn default_test_command(&self) -> Option<&'static str>;
    fn default_lint_command(&self) -> Option<&'static str>;
}
