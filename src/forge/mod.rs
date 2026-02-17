pub mod bitbucket;
pub mod gitea;
pub mod github;
pub mod gitlab;
pub mod traits;

pub type MrId = String;
pub type IssueId = String;

#[derive(Debug, Clone)]
pub struct User {
    pub id: Option<u64>,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub id: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct CheckRun {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct MergeRequest {
    pub id: MrId,
    pub iid: u64,
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    pub state: MrState,
    pub url: String,
    pub ci_status: Option<CiStatus>,
    pub approvals: Vec<User>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MrState {
    Open,
    Merged,
    Closed,
    Draft,
}

#[derive(Debug, Clone)]
pub struct CiStatus {
    pub state: CiState,
    pub pipelines: Vec<Pipeline>,
    pub checks: Vec<CheckRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiState {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub id: IssueId,
    pub iid: u64,
    pub title: String,
    pub url: String,
    pub state: IssueState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueState {
    Open,
    Closed,
}

pub fn client_from_forge_config(
    config: &crate::config::ForgeConfig,
) -> crate::error::Result<Box<dyn traits::Forge>> {
    let host = config
        .host
        .clone()
        .or_else(|| default_host_for_forge_type(&config.forge_type))
        .ok_or_else(|| {
            crate::error::HarmoniaError::Other(anyhow::anyhow!(format!(
                "forge host is required for '{}'",
                config.forge_type
            )))
        })?;
    let token = forge_token_from_sources(
        config.token.as_deref(),
        std::env::var("HARMONIA_FORGE_TOKEN").ok(),
    )
    .ok_or_else(|| {
        crate::error::HarmoniaError::Other(anyhow::anyhow!(
            "forge token is required (set HARMONIA_FORGE_TOKEN or configure [forge].token)"
        ))
    })?;

    match config.forge_type.as_str() {
        "github" => Ok(Box::new(github::GitHubClient::new(
            host,
            token,
            config.default_group.clone(),
        ))),
        "gitlab" => Ok(Box::new(gitlab::GitLabClient::new(
            host,
            token,
            config.default_group.clone(),
        ))),
        other => Err(crate::error::HarmoniaError::Other(anyhow::anyhow!(
            format!("forge '{}' is not implemented yet", other)
        ))),
    }
}

fn default_host_for_forge_type(forge_type: &str) -> Option<String> {
    match forge_type {
        "gitlab" => Some("gitlab.com".to_string()),
        "github" => Some("github.com".to_string()),
        _ => None,
    }
}

fn forge_token_from_sources(
    config_token: Option<&str>,
    env_token: Option<String>,
) -> Option<String> {
    let env_token = env_token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty());
    if env_token.is_some() {
        return env_token;
    }

    config_token
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use crate::config::ForgeConfig;
    use crate::forge::{client_from_forge_config, forge_token_from_sources};

    #[test]
    fn creates_github_client_from_config() {
        let config = ForgeConfig {
            forge_type: "github".to_string(),
            host: None,
            default_group: Some("example-org".to_string()),
            token: Some("token".to_string()),
        };
        let client = client_from_forge_config(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn creates_gitlab_client_from_config() {
        let config = ForgeConfig {
            forge_type: "gitlab".to_string(),
            host: None,
            default_group: None,
            token: Some("token".to_string()),
        };
        let client = client_from_forge_config(&config);
        assert!(client.is_ok());
    }

    #[test]
    fn errors_without_token() {
        let config = ForgeConfig {
            forge_type: "gitlab".to_string(),
            host: None,
            default_group: None,
            token: None,
        };
        let client = client_from_forge_config(&config);
        assert!(client.is_err());
    }

    #[test]
    fn env_token_takes_precedence_over_config_token() {
        let token = forge_token_from_sources(Some("config-token"), Some("env-token".to_string()));
        assert_eq!(token.as_deref(), Some("env-token"));
    }

    #[test]
    fn falls_back_to_config_token_when_env_missing() {
        let token = forge_token_from_sources(Some("config-token"), None);
        assert_eq!(token.as_deref(), Some("config-token"));
    }
}
