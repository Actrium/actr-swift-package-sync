//! 存储层错误定义

use thiserror::Error;

/// 存储层错误类型
#[derive(Error, Debug)]
pub enum StorageError {
    /// 数据库连接错误
    #[error("Database connection error: {0}")]
    ConnectionError(String),

    /// 查询执行错误
    #[error("Query execution error: {0}")]
    QueryError(String),

    /// 序列化错误
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// 反序列化错误
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// 数据完整性错误
    #[error("Data integrity error: {0}")]
    IntegrityError(String),

    /// 并发冲突错误
    #[error("Concurrency conflict: {0}")]
    ConcurrencyError(String),

    /// 资源不存在错误
    #[error("Resource not found: {0}")]
    NotFoundError(String),

    /// 配置错误
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// IO 错误
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// 其他错误
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

/// 存储层结果类型
pub type StorageResult<T> = Result<T, StorageError>;

/// 从 actor 错误转换为存储错误
impl From<actr_protocol::ActrError> for StorageError {
    fn from(err: actr_protocol::ActrError) -> Self {
        StorageError::Other(anyhow::anyhow!("Actor error: {err}"))
    }
}

impl From<rusqlite::Error> for StorageError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::SqliteFailure(sqlite_err, msg) => {
                let message = format!(
                    "SQLite error: {:?} - {}",
                    sqlite_err.code,
                    msg.unwrap_or_default()
                );
                match sqlite_err.code {
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                        StorageError::ConcurrencyError(message)
                    }
                    rusqlite::ErrorCode::ConstraintViolation => {
                        StorageError::IntegrityError(message)
                    }
                    rusqlite::ErrorCode::NotFound => StorageError::NotFoundError(message),
                    _ => StorageError::QueryError(message),
                }
            }
            _ => StorageError::QueryError(err.to_string()),
        }
    }
}
