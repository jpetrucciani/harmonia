use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceSettings {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_repos_dir")]
    pub repos_dir: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            name: String::new(),
            repos_dir: default_repos_dir(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub workspace: WorkspaceSettings,
    #[serde(default)]
    pub forge: Option<ForgeConfig>,
    #[serde(default)]
    pub repos: HashMap<String, RepoEntry>,
    #[serde(default)]
    pub groups: Option<GroupsConfig>,
    #[serde(default)]
    pub defaults: Option<DefaultsConfig>,
    #[serde(default)]
    pub hooks: Option<HooksConfig>,
    #[serde(default)]
    pub mr: Option<MrConfig>,
    #[serde(default)]
    pub versioning: Option<VersioningConfig>,
    #[serde(default)]
    pub changesets: Option<ChangesetsConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForgeConfig {
    #[serde(rename = "type")]
    pub forge_type: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub default_group: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoEntry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub external: bool,
    #[serde(default)]
    pub ignored: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GroupsConfig {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(flatten)]
    pub groups: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DefaultsConfig {
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub clone_protocol: Option<String>,
    #[serde(default)]
    pub clone_depth: Option<String>,
    #[serde(default)]
    pub include_untracked: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HooksConfig {
    #[serde(default)]
    pub pre_commit: Option<String>,
    #[serde(default)]
    pub pre_push: Option<String>,
    #[serde(default)]
    pub post_mr_create: Option<String>,
    #[serde(default)]
    pub custom: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MrConfig {
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub link_strategy: Option<String>,
    #[serde(default)]
    pub create_tracking_issue: Option<bool>,
    #[serde(default)]
    pub issue_template: Option<String>,
    #[serde(default)]
    pub add_trailers: Option<bool>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub require_tests: Option<bool>,
    #[serde(default)]
    pub draft: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersioningConfig {
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub bump_mode: Option<String>,
    #[serde(default)]
    pub calver_format: Option<String>,
    #[serde(default)]
    pub cascade_bumps: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChangesetsConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub dir: Option<String>,
}

fn default_repos_dir() -> String {
    "repos".to_string()
}
