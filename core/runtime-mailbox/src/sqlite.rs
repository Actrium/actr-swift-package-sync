//! SQLite 存储后端实现

use crate::{
    error::StorageResult,
    mailbox::{Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// SQLite 配置
#[derive(Debug, Clone)]
pub struct SqliteConfig {
    /// 数据库文件路径
    pub database_path: PathBuf,
    /// 是否启用 WAL 模式
    pub enable_wal: bool,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            database_path: PathBuf::from("mailbox.db"),
            enable_wal: true,
        }
    }
}

/// SQLite 连接包装器
struct SqliteConnection {
    conn: Mutex<Connection>,
}

impl SqliteConnection {
    fn new(config: &SqliteConfig) -> StorageResult<Self> {
        let conn = Connection::open(&config.database_path)?;
        if config.enable_wal {
            conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        }
        Self::create_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn create_tables(conn: &Connection) -> StorageResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                from_actr_id BLOB NOT NULL,  -- ActrId Protobuf bytes (所有消息必有 sender)
                payload BLOB NOT NULL,
                priority INTEGER NOT NULL,
                status INTEGER NOT NULL DEFAULT 0, -- 0: Queued, 1: Inflight
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_priority_status ON messages(priority DESC, status, created_at ASC);
            "#,
        )?;
        Ok(())
    }
}

/// SQLite 邮箱实现
pub struct SqliteMailbox {
    connection: Arc<SqliteConnection>,
}

impl SqliteMailbox {
    pub async fn new<P: AsRef<Path>>(database_path: P) -> StorageResult<Self> {
        let config = SqliteConfig {
            database_path: database_path.as_ref().to_path_buf(),
            ..Default::default()
        };
        Self::with_config(config).await
    }

    pub async fn with_config(config: SqliteConfig) -> StorageResult<Self> {
        let connection = Arc::new(SqliteConnection::new(&config)?);
        Ok(Self { connection })
    }
}

const DEFAULT_BATCH_SIZE: u32 = 32;

#[async_trait]
impl Mailbox for SqliteMailbox {
    async fn enqueue(
        &self,
        from: Vec<u8>,
        payload: Vec<u8>,
        priority: MessagePriority,
    ) -> StorageResult<Uuid> {
        let id = Uuid::new_v4();

        // from 已经是 Protobuf bytes，直接存储
        let conn = self.connection.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO messages (id, from_actr_id, payload, priority, status, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![
                id.to_string(),
                from,
                payload,
                priority as i64,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(id)
    }

    async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>> {
        let conn = self.connection.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
            UPDATE messages
            SET status = 1 -- Inflight
            WHERE id IN (
                SELECT id FROM messages
                WHERE status = 0 -- Queued
                ORDER BY priority DESC, created_at ASC
                LIMIT ?1
            )
            RETURNING id, from_actr_id, payload, priority, created_at, status;
            "#,
        )?;

        let mut messages = stmt
            .query_map(params![DEFAULT_BATCH_SIZE], |row| {
                // from_actr_id 直接返回 bytes，不反序列化
                let from: Vec<u8> = row.get(1)?;

                let priority_val: i64 = row.get(3)?;
                Ok(MessageRecord {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                    from,
                    payload: row.get(2)?,
                    priority: if priority_val == 1 {
                        MessagePriority::High
                    } else {
                        MessagePriority::Normal
                    },
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    status: if row.get::<_, i64>(5)? == 1 {
                        MessageStatus::Inflight
                    } else {
                        MessageStatus::Queued
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // The order of rows from a RETURNING clause is not guaranteed.
        // We must sort in memory to ensure priority is respected.
        messages.sort_unstable_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        Ok(messages)
    }

    async fn ack(&self, message_id: Uuid) -> StorageResult<()> {
        let conn = self.connection.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM messages WHERE id = ?1",
            params![message_id.to_string()],
        )?;
        Ok(())
    }

    async fn status(&self) -> StorageResult<MailboxStats> {
        let conn = self.connection.conn.lock().unwrap();
        let queued_messages: u64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE status = 0",
            [],
            |row| row.get(0),
        )?;
        let inflight_messages: u64 = conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE status = 1",
            [],
            |row| row.get(0),
        )?;

        let mut queued_by_priority = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT priority, COUNT(*) FROM messages WHERE status = 0 GROUP BY priority",
        )?;
        let rows = stmt.query_map([], |row| {
            let priority_val: i64 = row.get(0)?;
            let count: u64 = row.get(1)?;
            Ok((priority_val, count))
        })?;

        for row in rows {
            let (priority_val, count) = row?;
            let priority = if priority_val == 1 {
                MessagePriority::High
            } else {
                MessagePriority::Normal
            };
            queued_by_priority.insert(priority, count);
        }

        Ok(MailboxStats {
            queued_messages,
            inflight_messages,
            queued_by_priority,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::prost::Message as ProstMessage;
    use actr_protocol::{ActrId, ActrType, Realm};
    use tempfile::tempdir;

    async fn setup_mailbox() -> SqliteMailbox {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");
        SqliteMailbox::new(&path).await.unwrap()
    }

    fn dummy_actr_id_bytes() -> Vec<u8> {
        let actr_id = ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: 1000,
            r#type: ActrType {
                manufacturer: "test".to_string(),
                name: "TestActor".to_string(),
                version: "v1".to_string(),
            },
        };
        let mut buf = Vec::new();
        actr_id.encode(&mut buf).unwrap();
        buf
    }

    #[tokio::test]
    async fn test_enqueue_dequeue_ack_workflow() {
        let mailbox = setup_mailbox().await;

        // 1. Enqueue
        let from = dummy_actr_id_bytes();
        let payload = b"hello".to_vec();
        let msg_id = mailbox
            .enqueue(from.clone(), payload.clone(), MessagePriority::Normal)
            .await
            .unwrap();

        // 2. Dequeue
        let messages = mailbox.dequeue().await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, msg_id);
        assert_eq!(messages[0].from, from);
        assert_eq!(messages[0].payload, payload);
        assert_eq!(messages[0].status, MessageStatus::Inflight);

        // 3. Dequeue again, should be empty
        let messages_again = mailbox.dequeue().await.unwrap();
        assert!(messages_again.is_empty());

        // 4. Ack
        mailbox.ack(msg_id).await.unwrap();

        // 5. Check status, should be empty
        let stats = mailbox.status().await.unwrap();
        assert_eq!(stats.queued_messages, 0);
        assert_eq!(stats.inflight_messages, 0);
    }

    #[tokio::test]
    async fn test_priority_order() {
        let mailbox = setup_mailbox().await;

        let from = dummy_actr_id_bytes();
        let normal_id = mailbox
            .enqueue(from.clone(), b"normal".to_vec(), MessagePriority::Normal)
            .await
            .unwrap();
        let high_id = mailbox
            .enqueue(from.clone(), b"high".to_vec(), MessagePriority::High)
            .await
            .unwrap();

        // Dequeue should return both messages, with the high priority one first.
        let messages = mailbox.dequeue().await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].id, high_id); // High priority first
        assert_eq!(messages[1].id, normal_id); // Normal priority second
    }

    #[tokio::test]
    async fn test_status_tracking() {
        let mailbox = setup_mailbox().await;

        let from = dummy_actr_id_bytes();
        mailbox
            .enqueue(from.clone(), b"msg1".to_vec(), MessagePriority::Normal)
            .await
            .unwrap();
        mailbox
            .enqueue(from.clone(), b"msg2".to_vec(), MessagePriority::Normal)
            .await
            .unwrap();
        mailbox
            .enqueue(from.clone(), b"msg3".to_vec(), MessagePriority::High)
            .await
            .unwrap();

        let initial_stats = mailbox.status().await.unwrap();
        assert_eq!(initial_stats.queued_messages, 3);
        assert_eq!(initial_stats.inflight_messages, 0);
        assert_eq!(
            initial_stats
                .queued_by_priority
                .get(&MessagePriority::Normal),
            Some(&2)
        );
        assert_eq!(
            initial_stats.queued_by_priority.get(&MessagePriority::High),
            Some(&1)
        );

        // Dequeue all available messages (since 3 < DEFAULT_BATCH_SIZE)
        let dequeued = mailbox.dequeue().await.unwrap();
        assert_eq!(dequeued.len(), 3);

        let after_dequeue_stats = mailbox.status().await.unwrap();
        assert_eq!(after_dequeue_stats.queued_messages, 0);
        assert_eq!(after_dequeue_stats.inflight_messages, 3);

        // Ack the first message (which should be the high priority one)
        mailbox.ack(dequeued[0].id).await.unwrap();

        let final_stats = mailbox.status().await.unwrap();
        assert_eq!(final_stats.queued_messages, 0);
        assert_eq!(final_stats.inflight_messages, 2);
    }
}
