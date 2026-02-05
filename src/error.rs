use thiserror::Error;

use crate::config::ConfigError;

#[derive(Debug, Error)]
pub enum HarmoniaError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("git error: {0}")]
    Git(#[source] anyhow::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, HarmoniaError>;
