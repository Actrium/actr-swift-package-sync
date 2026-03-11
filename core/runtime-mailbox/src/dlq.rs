//! Dead Letter Queue (DLQ)
//!
//! DLQ stores poison messages that cannot be processed due to corruption or decoding failures.
//! Unlike the regular mailbox, DLQ messages are preserved indefinitely for manual intervention
//! and debugging.
//!
//! ## Design Principles
//!
//! - **Poison message classification**: Only DecodeFailure and severe corruption
//! - **Forensic data preservation**: Store raw bytes, error context, trace_id
//! - **Manual intervention workflow**: Redrive API after fixes
//! - **No auto-retry**: Poison messages don't retry automatically
//!
//! ## Workflow
//!
//! 1. **Framework detects poison message** (Protobuf decode failure)
//! 2. **Write to DLQ** with full context (raw_bytes, error_message, trace_id, etc.)
//! 3. **Alert operators** via metrics/logging (severity = 9)
//! 4. **Manual investigation** using DLQ query APIs
//! 5. **Fix and redrive** after resolving root cause (schema update, etc.)

use crate::error::StorageResult;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Dead Letter Queue record
///
/// Stores complete forensic information for poison messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlqRecord {
    /// Unique DLQ entry ID
    pub id: Uuid,

    /// Original message ID (if available from envelope)
    pub original_message_id: Option<String>,

    /// Sender ActrId (Protobuf bytes, if decodable from envelope)
    ///
    /// May be None if even the envelope is corrupted.
    pub from: Option<Vec<u8>>,

    /// Target ActrId (Protobuf bytes, if this was intended for specific Actor)
    pub to: Option<Vec<u8>>,

    /// Raw message bytes (complete original data for forensic analysis)
    pub raw_bytes: Vec<u8>,

    /// Error message describing why this is poison
    pub error_message: String,

    /// Error category (e.g., "protobuf_decode", "invalid_envelope", "corrupted_data")
    pub error_category: String,

    /// Distributed trace ID (for correlating with logs)
    pub trace_id: String,

    /// Request ID (if available)
    pub request_id: Option<String>,

    /// Timestamp when message was added to DLQ
    pub created_at: DateTime<Utc>,

    /// Number of redrive attempts (incremented on each redrive failure)
    pub redrive_attempts: u32,

    /// Last redrive attempt timestamp
    pub last_redrive_at: Option<DateTime<Utc>>,

    /// Additional context (JSON-encoded metadata like transport type, connection ID, etc.)
    pub context: Option<String>,
}

/// DLQ statistics
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DlqStats {
    /// Total messages in DLQ
    pub total_messages: u64,

    /// Messages by error category
    pub messages_by_category: std::collections::HashMap<String, u64>,

    /// Messages with redrive attempts > 0
    pub messages_with_redrive_attempts: u64,

    /// Oldest message timestamp
    pub oldest_message_at: Option<DateTime<Utc>>,
}

/// Query filter for DLQ records
#[derive(Debug, Clone, Default)]
pub struct DlqQuery {
    /// Filter by error category
    pub error_category: Option<String>,

    /// Filter by trace_id
    pub trace_id: Option<String>,

    /// Filter by sender ActrId (exact match on bytes)
    pub from: Option<Vec<u8>>,

    /// Maximum number of records to return
    pub limit: Option<u32>,

    /// Return only messages created after this timestamp
    pub created_after: Option<DateTime<Utc>>,
}

/// Dead Letter Queue interface
///
/// Provides persistence and query capabilities for poison messages.
#[async_trait]
pub trait DeadLetterQueue: Send + Sync {
    /// Add a poison message to DLQ
    ///
    /// # Parameters
    ///
    /// - `record`: Complete DLQ record with forensic data
    async fn enqueue(&self, record: DlqRecord) -> StorageResult<Uuid>;

    /// Query DLQ records with filtering
    ///
    /// # Parameters
    ///
    /// - `query`: Filter criteria for records
    async fn query(&self, query: DlqQuery) -> StorageResult<Vec<DlqRecord>>;

    /// Get a single DLQ record by ID
    async fn get(&self, id: Uuid) -> StorageResult<Option<DlqRecord>>;

    /// Delete a DLQ record (after manual resolution)
    ///
    /// # Parameters
    ///
    /// - `id`: DLQ record ID to delete
    async fn delete(&self, id: Uuid) -> StorageResult<()>;

    /// Increment redrive attempt counter
    ///
    /// Called when attempting to reprocess a DLQ message.
    async fn record_redrive_attempt(&self, id: Uuid) -> StorageResult<()>;

    /// Get DLQ statistics
    async fn stats(&self) -> StorageResult<DlqStats>;
}
