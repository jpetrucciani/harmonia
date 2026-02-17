use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::config::resolve::{load_repo_config, load_workspace_config, resolve_workspace};
use crate::config::{ConfigError, WorkspaceConfig};
use crate::core::repo::{Repo, RepoId};
use crate::ecosystem::EcosystemId;
use crate::graph::builder::build_graph;
use crate::graph::DependencyGraph;

#[derive(Debug)]
pub struct Workspace {
    pub root: PathBuf,
    pub config: WorkspaceConfig,
    pub repos: HashMap<RepoId, Repo>,
    pub graph: DependencyGraph,
}

impl Workspace {
    pub fn discover(start: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let resolved = resolve_workspace(start)?;
        Self::load_from(resolved.root, resolved.config_path)
    }

    pub fn load_from(root: PathBuf, config_path: PathBuf) -> Result<Self, ConfigError> {
        let mut config = load_workspace_config(&config_path)?;
        apply_env_overrides(&mut config);
        validate_workspace_config(&config)?;

        let repos = build_repos(&root, &config)?;
        let graph = build_graph(&repos).unwrap_or_else(|_| DependencyGraph::new());

        Ok(Self {
            root,
            config,
            repos,
            graph,
        })
    }
}

fn apply_env_overrides(config: &mut WorkspaceConfig) {
    if let Ok(repos_dir) = env::var("HARMONIA_REPOS_DIR") {
        config.workspace.repos_dir = repos_dir;
    }
}

fn validate_workspace_config(config: &WorkspaceConfig) -> Result<(), ConfigError> {
    if let Some(defaults) = config.defaults.as_ref() {
        if let Some(protocol) = defaults.clone_protocol.as_deref() {
            let protocol = protocol.trim().to_ascii_lowercase();
            if protocol != "ssh" && protocol != "https" {
                return Err(ConfigError::Validation(format!(
                    "defaults.clone_protocol must be 'ssh' or 'https', got '{}'",
                    protocol
                )));
            }
        }
    }

    if let Some(mr) = config.mr.as_ref() {
        if let Some(link_strategy) = mr.link_strategy.as_deref() {
            let link_strategy = link_strategy.trim().to_ascii_lowercase();
            if !matches!(
                link_strategy.as_str(),
                "related" | "description" | "issue" | "all"
            ) {
                return Err(ConfigError::Validation(format!(
                    "mr.link_strategy must be one of related, description, issue, all, got '{}'",
                    link_strategy
                )));
            }
        }
    }

    if let Some(changesets) = config.changesets.as_ref() {
        if let Some(enabled) = changesets.enabled {
            if enabled
                && changesets
                    .dir
                    .as_deref()
                    .is_some_and(|dir| dir.trim().is_empty())
            {
                return Err(ConfigError::Validation(
                    "changesets.dir cannot be empty when changesets.enabled=true".to_string(),
                ));
            }
        }
    }

    for (repo, entry) in &config.repos {
        if entry.external && entry.ignored {
            return Err(ConfigError::Validation(format!(
                "repo '{}' cannot set both external=true and ignored=true",
                repo
            )));
        }
    }

    Ok(())
}

fn build_repos(
    root: &Path,
    config: &WorkspaceConfig,
) -> Result<HashMap<RepoId, Repo>, ConfigError> {
    let mut repos = HashMap::new();
    let repos_dir = if config.workspace.repos_dir.is_empty() {
        "repos"
    } else {
        config.workspace.repos_dir.as_str()
    };

    for (repo_key, entry) in &config.repos {
        let repo_id = RepoId::new(repo_key.clone());
        let repo_path = root.join(repos_dir).join(repo_key);
        let repo_config = load_repo_config(&repo_path.join(".harmonia.toml"))?;
        let default_branch = entry
            .default_branch
            .clone()
            .or_else(|| {
                config
                    .defaults
                    .as_ref()
                    .and_then(|d| d.default_branch.clone())
            })
            .unwrap_or_else(|| "main".to_string());
        let remote_url = entry
            .url
            .clone()
            .or_else(|| build_default_url(config, repo_key));
        let repo_package_name = entry
            .package_name
            .clone()
            .or_else(|| {
                repo_config
                    .as_ref()
                    .and_then(|cfg| cfg.package.as_ref())
                    .and_then(|pkg| pkg.name.clone())
            })
            .or_else(|| Some(repo_key.clone()));
        let ecosystem = repo_config
            .as_ref()
            .and_then(|cfg| cfg.package.as_ref())
            .and_then(|pkg| pkg.ecosystem.as_ref())
            .and_then(|value| parse_ecosystem(value.as_str()));

        let repo = Repo {
            id: repo_id.clone(),
            path: repo_path,
            remote_url: remote_url.unwrap_or_default(),
            default_branch,
            package_name: repo_package_name,
            depends_on: entry.depends_on.clone(),
            ecosystem,
            config: repo_config,
            external: entry.external,
            ignored: entry.ignored,
        };
        repos.insert(repo_id, repo);
    }

    Ok(repos)
}

fn parse_ecosystem(value: &str) -> Option<EcosystemId> {
    match value {
        "python" => Some(EcosystemId::Python),
        "rust" => Some(EcosystemId::Rust),
        "node" => Some(EcosystemId::Node),
        "go" => Some(EcosystemId::Go),
        "java" => Some(EcosystemId::Java),
        other => Some(EcosystemId::Custom(other.to_string())),
    }
}

fn build_default_url(config: &WorkspaceConfig, repo_key: &str) -> Option<String> {
    let forge = config.forge.as_ref()?;
    let group = forge.default_group.as_ref()?;
    let host = forge
        .host
        .clone()
        .or_else(|| default_host_for_forge(&forge.forge_type))?;
    let protocol = config
        .defaults
        .as_ref()
        .and_then(|defaults| defaults.clone_protocol.clone())
        .unwrap_or_else(|| "ssh".to_string());

    let path = format!("{group}/{repo_key}.git");
    if protocol == "https" {
        Some(format!("https://{host}/{path}"))
    } else {
        Some(format!("git@{host}:{path}"))
    }
}

fn default_host_for_forge(forge_type: &str) -> Option<String> {
    match forge_type {
        "github" => Some("github.com".to_string()),
        "gitlab" => Some("gitlab.com".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{MrConfig, RepoEntry, WorkspaceConfig};
    use crate::core::workspace::validate_workspace_config;

    #[test]
    fn rejects_invalid_link_strategy() {
        let config = WorkspaceConfig {
            mr: Some(MrConfig {
                link_strategy: Some("invalid".to_string()),
                ..MrConfig::default()
            }),
            ..WorkspaceConfig::default()
        };

        let err = validate_workspace_config(&config).expect_err("should reject config");
        assert!(format!("{}", err).contains("mr.link_strategy"));
    }

    #[test]
    fn rejects_external_and_ignored_repo() {
        let mut config = WorkspaceConfig::default();
        config.repos.insert(
            "svc".to_string(),
            RepoEntry {
                external: true,
                ignored: true,
                ..RepoEntry::default()
            },
        );

        let err = validate_workspace_config(&config).expect_err("should reject config");
        assert!(format!("{}", err).contains("external=true and ignored=true"));
    }
}
