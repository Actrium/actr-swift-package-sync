//! DataStreamRegistry - Fast path data stream registry

use actr_protocol::{ActorResult, ActrId, DataStream};
use dashmap::DashMap;
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// Stream chunk callback type
///
/// # Design Rationale
/// Fast path is stream-based push, not RPC, so it doesn't need full Context:
/// - Only passes sender ActrId (to know where data comes from)
/// - Doesn't pass Context (avoids confusing RPC and Stream semantics)
/// - If reverse signaling needed, user should send via OutboundGate
pub type DataStreamCallback =
    Arc<dyn Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

/// DataStreamRegistry - Stream chunk callback manager
///
/// # Responsibilities
/// - Receive DataStream from LatencyFirst Lane (stream-format data packets)
/// - Maintain stream_id → callback mapping
/// - Concurrently invoke user-registered data stream callbacks
///
/// # Typical Use Cases
/// - Streaming RPC (server-side push streams)
/// - Real-time collaborative editing (multi-user editing sync)
/// - Game state streams (position updates, event streams)
/// - Log streams, sensor data streams, metrics streams
pub struct DataStreamRegistry {
    /// Concurrent mapping of stream_id → callback function
    callbacks: DashMap<String, DataStreamCallback>,
}

impl Default for DataStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DataStreamRegistry {
    pub fn new() -> Self {
        Self {
            callbacks: DashMap::new(),
        }
    }

    /// Register stream callback
    ///
    /// # Arguments
    /// - `stream_id`: stream identifier (must be globally unique)
    /// - `callback`: data stream handler callback
    pub fn register(&self, stream_id: String, callback: DataStreamCallback) {
        self.callbacks.insert(stream_id.clone(), callback);
        tracing::info!("📡 Registered data stream handler: {}", stream_id);
    }

    /// Unregister stream callback
    ///
    /// # Arguments
    /// - `stream_id`: stream identifier to unregister
    pub fn unregister(&self, stream_id: &str) {
        self.callbacks.remove(stream_id);
        tracing::info!("🚫 Unregistered data stream handler: {}", stream_id);
    }

    /// Dispatch data stream to callback (concurrent execution)
    ///
    /// # Arguments
    /// - `chunk`: data stream
    /// - `sender_id`: sender ActrId
    ///
    /// # Performance
    /// - Direct callback invocation, no queueing overhead
    /// - Latency: ~10μs
    /// - Concurrent execution, doesn't block other streams
    pub async fn dispatch(&self, chunk: DataStream, sender_id: ActrId) {
        let start = std::time::Instant::now();

        if let Some(callback) = self.callbacks.get(&chunk.stream_id) {
            let callback = callback.clone();
            tokio::spawn(async move {
                if let Err(e) = callback(chunk, sender_id).await {
                    tracing::error!("❌ Stream chunk callback error: {:?}", e);
                }
            });

            tracing::debug!("🚀 Dispatched data stream in {:?}", start.elapsed());
        } else {
            tracing::warn!("⚠️ No callback registered for stream: {}", chunk.stream_id);
        }
    }

    /// Get active stream count
    pub fn active_streams(&self) -> usize {
        self.callbacks.len()
    }
}
