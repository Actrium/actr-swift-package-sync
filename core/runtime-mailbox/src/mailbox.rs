//! # Actor Mailbox
//!
//! 本模块定义了持久化消息队列的核心接口和数据结构。
//!
//! ## 可靠队列工作流
//!
//! 本接口被设计为一个可靠队列，以防止消费者在处理消息时因崩溃而导致消息丢失。
//! 工作流如下:
//!
//! 1.  **`dequeue()`**: 消费者从队列中获取一批消息。这些消息在数据库中被原子性地标记为 `Inflight` (处理中)，但**不会被删除**。
//! 2.  **处理消息**: 消费者在本地处理这些消息。
//! 3.  **`ack()`**: 当一条消息被成功处理后，消费者调用 `ack(message_id)`。这会**永久删除**该消息，标志着工作单元的成功完成。
//!
//! 如果消费者在 `dequeue` 之后、`ack` 之前崩溃，那些处于 `Inflight` 状态的消息会保留在数据库中。
//! 下次消费者重启时，可以实现一个“清理”逻辑来重新处理这些“卡住”的消息。

use crate::error::StorageResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 消息优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MessagePriority {
    Normal,
    High,
}

/// 从队列中取出的消息记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// 消息 ID
    pub id: Uuid,
    /// 消息发送方的 ActrId (Protobuf bytes)
    ///
    /// # 设计说明
    /// - from 存储原始 Protobuf bytes，不反序列化为 ActrId 结构
    /// - 避免 decode → ActrId → encode 的循环
    /// - 只在真正需要使用时才反序列化一次
    /// - Gateway 直接传递 bytes，零开销
    /// - 所有进入 Mailbox 的消息都来自 WebRTC，必然有 sender
    pub from: Vec<u8>,
    /// 消息内容（raw bytes，不解包）
    pub payload: Vec<u8>,
    /// 优先级
    pub priority: MessagePriority,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 处理状态
    pub status: MessageStatus,
}

/// 消息处理状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    Queued,
    Inflight,
}

/// 邮箱的统计信息
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxStats {
    /// 在队列中等待处理的消息总数
    pub queued_messages: u64,
    /// 已出队但尚未被确认(ack)的消息总数
    pub inflight_messages: u64,
    /// 按优先级的排队消息数
    pub queued_by_priority: std::collections::HashMap<MessagePriority, u64>,
}

/// 邮箱接口 - 定义消息持久化的核心操作
///
/// ## 使用示例: `dequeue -> process -> ack` 循环
///
/// `dequeue` 方法会自动获取下一批消息。调用者无需关心批量大小，这个细节由实现内部处理。
///
/// ```rust,no_run
/// use actr_runtime_mailbox::prelude::*;
/// use std::time::Duration;
///
/// async fn message_processor(mailbox: impl Mailbox) {
///     loop {
///         // 1. 从队列中获取下一批消息
///         match mailbox.dequeue().await {
///             Ok(messages) => {
///                 if messages.is_empty() {
///                     tokio::time::sleep(Duration::from_secs(1)).await;
///                     continue;
///                 }
///
///                 // 2. 逐条处理消息
///                 for msg in messages {
///                     println!("Processing message: {}", msg.id);
///                     // ... 在这里执行你的业务逻辑 ...
///
///                     // 3. 成功处理后，确认这一条消息
///                     if let Err(e) = mailbox.ack(msg.id).await {
///                         eprintln!("消息 {} 确认失败: {}", msg.id, e);
///                     }
///                 }
///             }
///             Err(e) => {
///                 eprintln!("从队列拉取消息失败: {}", e);
///                 tokio::time::sleep(Duration::from_secs(5)).await; // 数据库错误，等待更长时间
///             }
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait Mailbox: Send + Sync {
    /// 将消息加入队列。
    ///
    /// # 参数
    /// - `from`: 消息发送方 ActrId (Protobuf bytes，由 Gateway 直接提供，不解包)
    /// - `payload`: 消息内容（raw bytes，不解包）
    /// - `priority`: 消息优先级
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> StorageResult<Uuid>;

    /// 从队列中取出一批消息。
    ///
    /// 此方法将自动处理优先级：只要有高优先级消息，就会优先返回它们。
    /// 取出的消息会被原子性地标记为 `Inflight` (处理中)，但不会被删除。
    /// 必须在处理完成后调用 `ack()` 来将其永久删除。
    async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>>;

    /// 确认一条消息已成功处理，将其从队列中永久删除。
    async fn ack(&self, message_id: Uuid) -> StorageResult<()>;

    /// 获取当前邮箱的统计信息。
    async fn status(&self) -> StorageResult<MailboxStats>;
}
