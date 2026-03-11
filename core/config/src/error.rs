//! Error types for actr-config

use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during configuration parsing and processing
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse TOML: {0}")]
    TomlParseError(#[from] toml::de::Error),

    #[error("Failed to serialize TOML: {0}")]
    TomlSerializeError(#[from] toml::ser::Error),

    #[error("Invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),

    #[error("Proto file not found: {0}")]
    ProtoFileNotFound(PathBuf, #[source] std::io::Error),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Unsupported config edition: {0}")]
    UnsupportedEdition(u32),

    #[error("Edition mismatch in inheritance: parent={parent}, child={child}")]
    EditionMismatch { parent: u32, child: u32 },

    #[error("Invalid ACL configuration: {0}")]
    InvalidAcl(String),

    #[error("Configuration validation failed: {0}")]
    ValidationError(String),

    #[error("Invalid ActrType format: {0}")]
    InvalidActrType(String),

    #[error("Invalid routing rule: {0}")]
    InvalidRoutingRule(String),

    #[error("Invalid dependency configuration: {0}")]
    InvalidDependency(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;

// Backward compatibility alias
#[deprecated(since = "0.2.0", note = "Use ConfigError instead")]
pub type ActrConfigError = ConfigError;
