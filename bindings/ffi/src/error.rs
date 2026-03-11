//! Error types for actr

/// Error type for actr operations
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ActrError {
    #[error("Configuration error: {msg}")]
    ConfigError { msg: String },

    #[error("Connection error: {msg}")]
    ConnectionError { msg: String },

    #[error("RPC error: {msg}")]
    RpcError { msg: String },

    #[error("State error: {msg}")]
    StateError { msg: String },

    #[error("Internal error: {msg}")]
    InternalError { msg: String },

    #[error("Timeout error: {msg}")]
    TimeoutError { msg: String },

    #[error("Workload error: {msg}")]
    WorkloadError { msg: String },
}

pub type ActrResult<T> = Result<T, ActrError>;

impl From<actr_protocol::ActrError> for ActrError {
    fn from(e: actr_protocol::ActrError) -> Self {
        match e {
            actr_protocol::ActrError::Unavailable(msg) => ActrError::ConnectionError { msg },
            actr_protocol::ActrError::TimedOut => ActrError::TimeoutError {
                msg: "operation timed out".to_string(),
            },
            actr_protocol::ActrError::NotFound(msg)
            | actr_protocol::ActrError::PermissionDenied(msg)
            | actr_protocol::ActrError::InvalidArgument(msg)
            | actr_protocol::ActrError::UnknownRoute(msg)
            | actr_protocol::ActrError::DecodeFailure(msg)
            | actr_protocol::ActrError::NotImplemented(msg)
            | actr_protocol::ActrError::Internal(msg) => ActrError::RpcError { msg },
            actr_protocol::ActrError::DependencyNotFound {
                service_name,
                message,
            } => ActrError::RpcError {
                msg: format!("dependency '{service_name}' not found: {message}"),
            },
        }
    }
}

impl From<ActrError> for actr_protocol::ActrError {
    fn from(e: ActrError) -> Self {
        match e {
            ActrError::ConfigError { msg } => actr_protocol::ActrError::InvalidArgument(msg),
            ActrError::ConnectionError { msg } => actr_protocol::ActrError::Unavailable(msg),
            ActrError::RpcError { msg }
            | ActrError::StateError { msg }
            | ActrError::InternalError { msg }
            | ActrError::WorkloadError { msg } => actr_protocol::ActrError::Internal(msg),
            ActrError::TimeoutError { .. } => actr_protocol::ActrError::TimedOut,
        }
    }
}
