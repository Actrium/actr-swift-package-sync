//! SQLite implementation of Dead Letter Queue

use crate::{
    dlq::{DeadLetterQueue, DlqQuery, DlqRecord, DlqStats},
    error::StorageResult,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

/// SQLite connection wrapper for DLQ
struct SqliteDlqConnection {
    conn: Mutex<Connection>,
}

impl SqliteDlqConnection {
    fn new(conn: Connection) -> StorageResult<Self> {
        Self::create_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn create_tables(conn: &Connection) -> StorageResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS dead_letter_queue (
                id TEXT PRIMARY KEY,
                original_message_id TEXT,
                from_actr_id BLOB,           -- Sender ActrId (Protobuf bytes, nullable)
                to_actr_id BLOB,             -- Target ActrId (Protobuf bytes, nullable)
                raw_bytes BLOB NOT NULL,     -- Complete original message for forensics
                error_message TEXT NOT NULL, -- Human-readable error description
                error_category TEXT NOT NULL,-- Error classification (e.g., "protobuf_decode")
                trace_id TEXT NOT NULL,      -- Distributed trace ID
                request_id TEXT,             -- Request ID (if available)
                created_at TEXT NOT NULL,    -- Timestamp of DLQ entry
                redrive_attempts INTEGER NOT NULL DEFAULT 0,
                last_redrive_at TEXT,        -- Last redrive attempt timestamp
                context TEXT                 -- JSON-encoded additional metadata
            );

            -- Index for common query patterns
            CREATE INDEX IF NOT EXISTS idx_dlq_created_at ON dead_letter_queue(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_dlq_error_category ON dead_letter_queue(error_category, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_dlq_trace_id ON dead_letter_queue(trace_id);
            "#,
        )?;
        Ok(())
    }
}

/// SQLite-backed Dead Letter Queue
pub struct SqliteDeadLetterQueue {
    connection: Arc<SqliteDlqConnection>,
}

impl SqliteDeadLetterQueue {
    /// Create a new DLQ using an existing database connection
    ///
    /// DLQ shares the same database file as mailbox but uses separate tables.
    pub fn new(conn: Connection) -> StorageResult<Self> {
        let connection = Arc::new(SqliteDlqConnection::new(conn)?);
        Ok(Self { connection })
    }

    /// Create a new DLQ with a separate database file
    pub async fn new_standalone<P: AsRef<Path>>(database_path: P) -> StorageResult<Self> {
        let conn = Connection::open(database_path.as_ref())?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        Self::new(conn)
    }
}

#[async_trait]
impl DeadLetterQueue for SqliteDeadLetterQueue {
    async fn enqueue(&self, record: DlqRecord) -> StorageResult<Uuid> {
        let id = record.id;
        let conn = self.connection.conn.lock().unwrap();

        conn.execute(
            r#"
            INSERT INTO dead_letter_queue (
                id, original_message_id, from_actr_id, to_actr_id, raw_bytes,
                error_message, error_category, trace_id, request_id,
                created_at, redrive_attempts, last_redrive_at, context
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                id.to_string(),
                record.original_message_id,
                record.from,
                record.to,
                record.raw_bytes,
                record.error_message,
                record.error_category,
                record.trace_id,
                record.request_id,
                record.created_at.to_rfc3339(),
                record.redrive_attempts,
                record.last_redrive_at.map(|dt| dt.to_rfc3339()),
                record.context,
            ],
        )?;

        Ok(id)
    }

    async fn query(&self, query: DlqQuery) -> StorageResult<Vec<DlqRecord>> {
        let conn = self.connection.conn.lock().unwrap();

        // Build dynamic query based on filters
        let mut sql = String::from(
            r#"
            SELECT id, original_message_id, from_actr_id, to_actr_id, raw_bytes,
                   error_message, error_category, trace_id, request_id,
                   created_at, redrive_attempts, last_redrive_at, context
            FROM dead_letter_queue
            WHERE 1=1
            "#,
        );

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref category) = query.error_category {
            sql.push_str(" AND error_category = ?");
            params_vec.push(Box::new(category.clone()));
        }

        if let Some(ref trace_id) = query.trace_id {
            sql.push_str(" AND trace_id = ?");
            params_vec.push(Box::new(trace_id.clone()));
        }

        if let Some(ref from_bytes) = query.from {
            sql.push_str(" AND from_actr_id = ?");
            params_vec.push(Box::new(from_bytes.clone()));
        }

        if let Some(ref created_after) = query.created_after {
            sql.push_str(" AND created_at > ?");
            params_vec.push(Box::new(created_after.to_rfc3339()));
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = query.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = conn.prepare(&sql)?;

        // Convert Vec<Box<dyn ToSql>> to params
        let params_refs: Vec<&dyn rusqlite::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();

        let records = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(DlqRecord {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                    original_message_id: row.get(1)?,
                    from: row.get(2)?,
                    to: row.get(3)?,
                    raw_bytes: row.get(4)?,
                    error_message: row.get(5)?,
                    error_category: row.get(6)?,
                    trace_id: row.get(7)?,
                    request_id: row.get(8)?,
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    redrive_attempts: row.get(10)?,
                    last_redrive_at: row.get::<_, Option<String>>(11)?.map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .unwrap()
                            .with_timezone(&Utc)
                    }),
                    context: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(records)
    }

    async fn get(&self, id: Uuid) -> StorageResult<Option<DlqRecord>> {
        let conn = self.connection.conn.lock().unwrap();

        let result = conn.query_row(
            r#"
            SELECT id, original_message_id, from_actr_id, to_actr_id, raw_bytes,
                   error_message, error_category, trace_id, request_id,
                   created_at, redrive_attempts, last_redrive_at, context
            FROM dead_letter_queue
            WHERE id = ?1
            "#,
            params![id.to_string()],
            |row| {
                Ok(DlqRecord {
                    id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                    original_message_id: row.get(1)?,
                    from: row.get(2)?,
                    to: row.get(3)?,
                    raw_bytes: row.get(4)?,
                    error_message: row.get(5)?,
                    error_category: row.get(6)?,
                    trace_id: row.get(7)?,
                    request_id: row.get(8)?,
                    created_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(9)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    redrive_attempts: row.get(10)?,
                    last_redrive_at: row.get::<_, Option<String>>(11)?.map(|s| {
                        DateTime::parse_from_rfc3339(&s)
                            .unwrap()
                            .with_timezone(&Utc)
                    }),
                    context: row.get(12)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, id: Uuid) -> StorageResult<()> {
        let conn = self.connection.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM dead_letter_queue WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    async fn record_redrive_attempt(&self, id: Uuid) -> StorageResult<()> {
        let conn = self.connection.conn.lock().unwrap();
        conn.execute(
            r#"
            UPDATE dead_letter_queue
            SET redrive_attempts = redrive_attempts + 1,
                last_redrive_at = ?1
            WHERE id = ?2
            "#,
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(())
    }

    async fn stats(&self) -> StorageResult<DlqStats> {
        let conn = self.connection.conn.lock().unwrap();

        let total_messages: u64 =
            conn.query_row("SELECT COUNT(*) FROM dead_letter_queue", [], |row| {
                row.get(0)
            })?;

        let messages_with_redrive_attempts: u64 = conn.query_row(
            "SELECT COUNT(*) FROM dead_letter_queue WHERE redrive_attempts > 0",
            [],
            |row| row.get(0),
        )?;

        let oldest_message_at: Option<DateTime<Utc>> = conn
            .query_row("SELECT MIN(created_at) FROM dead_letter_queue", [], |row| {
                row.get::<_, Option<String>>(0)
            })?
            .map(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .unwrap()
                    .with_timezone(&Utc)
            });

        // Group by error category
        let mut messages_by_category = HashMap::new();
        let mut stmt = conn.prepare(
            "SELECT error_category, COUNT(*) FROM dead_letter_queue GROUP BY error_category",
        )?;
        let rows = stmt.query_map([], |row| {
            let category: String = row.get(0)?;
            let count: u64 = row.get(1)?;
            Ok((category, count))
        })?;

        for row in rows {
            let (category, count) = row?;
            messages_by_category.insert(category, count);
        }

        Ok(DlqStats {
            total_messages,
            messages_by_category,
            messages_with_redrive_attempts,
            oldest_message_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn setup_dlq() -> SqliteDeadLetterQueue {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_dlq.db");
        SqliteDeadLetterQueue::new_standalone(&path).await.unwrap()
    }

    fn dummy_dlq_record() -> DlqRecord {
        DlqRecord {
            id: Uuid::new_v4(),
            original_message_id: Some("msg-123".to_string()),
            from: Some(vec![1, 2, 3]),
            to: Some(vec![4, 5, 6]),
            raw_bytes: vec![0xDE, 0xAD, 0xBE, 0xEF],
            error_message: "Protobuf decode failed: unexpected EOF".to_string(),
            error_category: "protobuf_decode".to_string(),
            trace_id: "trace-abc-123".to_string(),
            request_id: Some("req-xyz".to_string()),
            created_at: Utc::now(),
            redrive_attempts: 0,
            last_redrive_at: None,
            context: Some(r#"{"transport": "webrtc"}"#.to_string()),
        }
    }

    #[tokio::test]
    async fn test_enqueue_and_get() {
        let dlq = setup_dlq().await;
        let record = dummy_dlq_record();
        let id = record.id;

        // Enqueue
        dlq.enqueue(record.clone()).await.unwrap();

        // Get
        let retrieved = dlq.get(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, id);
        assert_eq!(retrieved.error_message, record.error_message);
        assert_eq!(retrieved.error_category, record.error_category);
        assert_eq!(retrieved.trace_id, record.trace_id);
    }

    #[tokio::test]
    async fn test_query_by_category() {
        let dlq = setup_dlq().await;

        let mut record1 = dummy_dlq_record();
        record1.error_category = "protobuf_decode".to_string();
        dlq.enqueue(record1).await.unwrap();

        let mut record2 = dummy_dlq_record();
        record2.error_category = "corrupted_envelope".to_string();
        dlq.enqueue(record2).await.unwrap();

        let mut record3 = dummy_dlq_record();
        record3.error_category = "protobuf_decode".to_string();
        dlq.enqueue(record3).await.unwrap();

        // Query by category
        let query = DlqQuery {
            error_category: Some("protobuf_decode".to_string()),
            ..Default::default()
        };
        let results = dlq.query(query).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| r.error_category == "protobuf_decode")
        );
    }

    #[tokio::test]
    async fn test_redrive_attempt_tracking() {
        let dlq = setup_dlq().await;
        let record = dummy_dlq_record();
        let id = record.id;

        dlq.enqueue(record).await.unwrap();

        // Initial state
        let retrieved = dlq.get(id).await.unwrap().unwrap();
        assert_eq!(retrieved.redrive_attempts, 0);
        assert!(retrieved.last_redrive_at.is_none());

        // Record redrive attempt
        dlq.record_redrive_attempt(id).await.unwrap();

        // Check updated state
        let updated = dlq.get(id).await.unwrap().unwrap();
        assert_eq!(updated.redrive_attempts, 1);
        assert!(updated.last_redrive_at.is_some());
    }

    #[tokio::test]
    async fn test_delete() {
        let dlq = setup_dlq().await;
        let record = dummy_dlq_record();
        let id = record.id;

        dlq.enqueue(record).await.unwrap();
        assert!(dlq.get(id).await.unwrap().is_some());

        dlq.delete(id).await.unwrap();
        assert!(dlq.get(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_stats() {
        let dlq = setup_dlq().await;

        let mut record1 = dummy_dlq_record();
        record1.error_category = "protobuf_decode".to_string();
        dlq.enqueue(record1.clone()).await.unwrap();

        let mut record2 = dummy_dlq_record();
        record2.error_category = "corrupted_envelope".to_string();
        dlq.enqueue(record2).await.unwrap();

        // Record a redrive attempt
        dlq.record_redrive_attempt(record1.id).await.unwrap();

        let stats = dlq.stats().await.unwrap();
        assert_eq!(stats.total_messages, 2);
        assert_eq!(stats.messages_with_redrive_attempts, 1);
        assert_eq!(
            stats
                .messages_by_category
                .get("protobuf_decode")
                .copied()
                .unwrap_or(0),
            1
        );
        assert_eq!(
            stats
                .messages_by_category
                .get("corrupted_envelope")
                .copied()
                .unwrap_or(0),
            1
        );
        assert!(stats.oldest_message_at.is_some());
    }
}
