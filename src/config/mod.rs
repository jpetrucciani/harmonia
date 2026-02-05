pub mod repo;
pub mod resolve;
pub mod workspace;

pub use repo::{
    CiConfig, DepsConfig, PackageConfig, RepoConfig, RepoHooksConfig, RepoVersioningConfig,
};
pub use workspace::{
    ChangesetsConfig, DefaultsConfig, ForgeConfig, GroupsConfig, HooksConfig, MrConfig, RepoEntry,
    VersioningConfig, WorkspaceConfig, WorkspaceSettings,
};

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("workspace not found")]
    WorkspaceNotFound,
    #[error("config file not found: {0}")]
    ConfigNotFound(PathBuf),
    #[error("invalid workspace root: {0}")]
    InvalidWorkspace(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config at {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

pub type Result<T> = std::result::Result<T, ConfigError>;
