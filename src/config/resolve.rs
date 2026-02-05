use std::env;
use std::path::{Path, PathBuf};

use crate::config::{ConfigError, RepoConfig, WorkspaceConfig};

#[derive(Debug, Clone)]
pub struct ResolvedWorkspace {
    pub root: PathBuf,
    pub config_path: PathBuf,
}

pub fn resolve_workspace(start: impl AsRef<Path>) -> Result<ResolvedWorkspace, ConfigError> {
    resolve_workspace_with_overrides(start, None, None)
}

pub fn resolve_workspace_with_overrides(
    start: impl AsRef<Path>,
    workspace_root: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<ResolvedWorkspace, ConfigError> {
    if let Some(root) = workspace_root {
        return resolve_with_root(root);
    }

    if let Some(config) = config_path {
        return resolve_with_config(config);
    }

    if let Ok(path) = env::var("HARMONIA_WORKSPACE") {
        let root = PathBuf::from(path);
        return resolve_with_root(root);
    }

    if let Ok(path) = env::var("HARMONIA_CONFIG") {
        let config_path = PathBuf::from(path);
        return resolve_with_config(config_path);
    }

    find_workspace_from(start.as_ref())
}

pub fn load_workspace_config(path: &Path) -> Result<WorkspaceConfig, ConfigError> {
    if !path.is_file() {
        return Err(ConfigError::ConfigNotFound(path.to_path_buf()));
    }

    let contents = std::fs::read_to_string(path)?;
    toml::from_str(&contents).map_err(|source| ConfigError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

pub fn load_repo_config(path: &Path) -> Result<Option<RepoConfig>, ConfigError> {
    if !path.is_file() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(path)?;
    let config = toml::from_str(&contents).map_err(|source| ConfigError::Toml {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(Some(config))
}

fn resolve_with_root(root: PathBuf) -> Result<ResolvedWorkspace, ConfigError> {
    if !root.is_dir() {
        return Err(ConfigError::InvalidWorkspace(root));
    }

    let config_path = root.join(".harmonia").join("config.toml");
    Ok(ResolvedWorkspace { root, config_path })
}

fn resolve_with_config(config_path: PathBuf) -> Result<ResolvedWorkspace, ConfigError> {
    let root = infer_root_from_config(&config_path)
        .ok_or_else(|| ConfigError::InvalidWorkspace(config_path.clone()))?;

    Ok(ResolvedWorkspace { root, config_path })
}

fn infer_root_from_config(config_path: &Path) -> Option<PathBuf> {
    let parent = config_path.parent()?;
    if parent.file_name()? == ".harmonia" {
        return parent.parent().map(|p| p.to_path_buf());
    }

    parent.parent().map(|p| p.to_path_buf())
}

fn find_workspace_from(start: &Path) -> Result<ResolvedWorkspace, ConfigError> {
    for ancestor in start.ancestors() {
        let config_path = ancestor.join(".harmonia").join("config.toml");
        if config_path.is_file() {
            return Ok(ResolvedWorkspace {
                root: ancestor.to_path_buf(),
                config_path,
            });
        }
    }

    Err(ConfigError::WorkspaceNotFound)
}
