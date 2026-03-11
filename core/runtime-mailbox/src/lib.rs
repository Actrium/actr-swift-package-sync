//! # Actor-RTC Mailbox Layer
//!
//! 🗄️ Actor-RTC 框架的持久化邮箱层，由 SQLite 支持。
//!
//! ## 核心功能
//!
//! - **消息持久化**: 可靠的消息队列和邮箱存储
//! - **Dead Letter Queue**: 毒消息隔离和人工干预
//!
//! ## 快速开始
//!
//! ```rust,no_run
//! use actr_runtime_mailbox::prelude::*;
//! use actr_protocol::{ActrId, Realm, ActrType};
//! use actr_protocol::prost::Message as ProstMessage;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 创建 SQLite 邮箱存储
//!     let mailbox = SqliteMailbox::new("./data/mailbox.db").await?;
//!
//!     // 创建发送者 ActrId 并序列化
//!     let sender = ActrId {
//!         realm: Realm { realm_id: 1 },
//!         serial_number: 1000,
//!         r#type: ActrType {
//!             manufacturer: "example".to_string(),
//!             name: "TestActor".to_string(),
//!             version: "v1".to_string(),
//!         },
//!     };
//!     let mut from_bytes = Vec::new();
//!     sender.encode(&mut from_bytes)?;
//!
//!     let message = b"Hello, World!".to_vec();
//!
//!     // 入队消息（from 为发送方 ActrId 的 Protobuf bytes）
//!     let message_id = mailbox.enqueue(from_bytes, message, MessagePriority::Normal).await?;
//!
//!     // 出队消息
//!     let messages = mailbox.dequeue().await?;
//!     println!("Retrieved {} messages", messages.len());
//!
//!     // 确认消息
//!     if let Some(msg) = messages.first() {
//!         mailbox.ack(msg.id).await?;
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod error;
pub mod mailbox;
pub mod sqlite;

// Dead Letter Queue modules
pub mod dlq;
pub mod sqlite_dlq;

// 重新导出核心类型
pub use actr_protocol::{ActrError, ActrId};

// 存储层核心接口
pub use error::{StorageError, StorageResult};
pub use mailbox::{Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus};

// DLQ 核心接口
pub use dlq::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats};

// 后端实现
pub use sqlite::{SqliteConfig, SqliteMailbox};
pub use sqlite_dlq::SqliteDeadLetterQueue;

pub mod prelude {
    //! 邮箱层常用类型和 trait 的便利导入

    pub use crate::error::{StorageError, StorageResult};
    pub use crate::mailbox::{
        Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
    };
    pub use crate::sqlite::{SqliteConfig, SqliteMailbox};

    // Dead Letter Queue
    pub use crate::dlq::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats};
    pub use crate::sqlite_dlq::SqliteDeadLetterQueue;

    // 基础类型
    pub use actr_protocol::{ActrError, ActrId};

    // 异步 trait 支持
    pub use async_trait::async_trait;

    // 常用工具
    pub use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;
}
