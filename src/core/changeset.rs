use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::WorkspaceConfig;
use crate::core::repo::RepoId;
use crate::error::{HarmoniaError, Result};
use crate::forge::{Issue, MergeRequest};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChangesetId(String);

impl ChangesetId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Changeset {
    pub id: ChangesetId,
    pub branch: String,
    pub repos: Vec<RepoId>,
    pub merge_order: Vec<RepoId>,
    pub mrs: HashMap<RepoId, MergeRequest>,
    pub tracking_issue: Option<Issue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChangesetFile {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub branch: String,
    #[serde(default)]
    pub repos: Vec<ChangesetRepoSummary>,
    #[serde(skip)]
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChangesetRepoSummary {
    pub repo: String,
    #[serde(default)]
    pub summary: String,
}

impl ChangesetFile {
    pub fn repo_set(&self) -> HashSet<RepoId> {
        self.repos
            .iter()
            .map(|entry| RepoId::new(entry.repo.clone()))
            .collect()
    }

    pub fn repo_summary_map(&self) -> HashMap<RepoId, String> {
        self.repos
            .iter()
            .map(|entry| (RepoId::new(entry.repo.clone()), entry.summary.clone()))
            .collect()
    }
}

pub fn load_changeset_files(
    workspace_root: &Path,
    config: &WorkspaceConfig,
) -> Result<Vec<ChangesetFile>> {
    if !changesets_enabled(config) {
        return Ok(Vec::new());
    }

    let dir = workspace_root.join(changesets_dir(config)?);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }

        let contents = fs::read_to_string(&path)?;
        let mut parsed: ChangesetFile = toml::from_str(&contents).map_err(|err| {
            HarmoniaError::Other(anyhow::anyhow!(format!(
                "failed to parse changeset {}: {}",
                path.display(),
                err
            )))
        })?;
        parsed.path = path;
        files.push(parsed);
    }

    files.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(files)
}

pub fn select_active_changeset(
    changesets: &[ChangesetFile],
    branches: &HashSet<String>,
) -> Result<Option<ChangesetFile>> {
    let mut matches = changesets
        .iter()
        .filter(|changeset| branches.contains(&changeset.branch))
        .cloned()
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() > 1 {
        matches.sort_by(|a, b| a.id.cmp(&b.id));
        let names = matches
            .iter()
            .map(|changeset| changeset.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(HarmoniaError::Other(anyhow::anyhow!(format!(
            "multiple changesets match current branches: {}",
            names
        ))));
    }

    Ok(matches.into_iter().next())
}

pub fn changesets_enabled(config: &WorkspaceConfig) -> bool {
    config
        .changesets
        .as_ref()
        .and_then(|changesets| changesets.enabled)
        .unwrap_or(false)
}

pub fn changesets_dir(config: &WorkspaceConfig) -> Result<PathBuf> {
    let configured = config
        .changesets
        .as_ref()
        .and_then(|changesets| changesets.dir.as_ref())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("changesets");

    let path = PathBuf::from(configured);
    if path.is_absolute() {
        return Ok(path);
    }

    Ok(PathBuf::from(".harmonia").join(path))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::core::changeset::{
        changesets_enabled, select_active_changeset, ChangesetFile, ChangesetRepoSummary,
    };

    #[test]
    fn active_changeset_selected_by_branch() {
        let changesets = vec![ChangesetFile {
            id: "cs-auth".to_string(),
            title: "auth".to_string(),
            description: String::new(),
            branch: "feature/auth".to_string(),
            repos: vec![ChangesetRepoSummary {
                repo: "app".to_string(),
                summary: String::new(),
            }],
            path: std::path::PathBuf::new(),
        }];

        let branches = HashSet::from(["feature/auth".to_string()]);
        let selected = select_active_changeset(&changesets, &branches)
            .expect("select changeset")
            .expect("changeset exists");
        assert_eq!(selected.id, "cs-auth");
    }

    #[test]
    fn changesets_disabled_by_default() {
        let config = crate::config::WorkspaceConfig::default();
        assert!(!changesets_enabled(&config));
    }
}
