use std::path::PathBuf;

use crate::config::RepoConfig;
use crate::core::version::VersionReq;
use crate::ecosystem::EcosystemId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoId(String);

impl RepoId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Repo {
    pub id: RepoId,
    pub path: PathBuf,
    pub remote_url: String,
    pub default_branch: String,
    pub package_name: Option<String>,
    pub ecosystem: Option<EcosystemId>,
    pub config: Option<RepoConfig>,
    pub external: bool,
    pub ignored: bool,
}

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub repo: RepoId,
    pub branch: String,
    pub ahead: usize,
    pub behind: usize,
    pub staged: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub untracked: Vec<PathBuf>,
    pub conflicts: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub constraint: VersionReq,
    pub is_internal: bool,
}
