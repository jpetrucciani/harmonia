use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoConfig {
    #[serde(default)]
    pub package: Option<PackageConfig>,
    #[serde(default)]
    pub versioning: Option<RepoVersioningConfig>,
    #[serde(default)]
    pub dependencies: Option<DepsConfig>,
    #[serde(default)]
    pub hooks: Option<RepoHooksConfig>,
    #[serde(default)]
    pub ci: Option<CiConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PackageConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub ecosystem: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoVersioningConfig {
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default)]
    pub bump_mode: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DepsConfig {
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub internal_pattern: Option<String>,
    #[serde(default)]
    pub internal_packages: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RepoHooksConfig {
    #[serde(default)]
    pub disable_workspace_hooks: Option<Vec<String>>,
    #[serde(default)]
    pub pre_commit: Option<String>,
    #[serde(default)]
    pub pre_push: Option<String>,
    #[serde(default)]
    pub custom: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CiConfig {
    #[serde(default)]
    pub required_checks: Option<Vec<String>>,
    #[serde(default)]
    pub timeout_minutes: Option<u64>,
}
