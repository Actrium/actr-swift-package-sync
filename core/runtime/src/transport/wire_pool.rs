//! WirePool - Wire connection pool manager
//!
//! Manages connection strategies: saturated concurrent connections, automatic retry, and fallback strategies.
//! Uses watch channels to broadcast connection status, implementing zero-polling event-driven architecture.

use super::backoff::ExponentialBackoff;
use super::error::NetworkResult;
use super::wire_handle::{WireHandle, WireStatus};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tokio::sync::{RwLock, watch};

/// Connection type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnType {
    WebSocket,
    WebRTC,
}

impl ConnType {
    /// Convert to array index
    const fn as_index(self) -> usize {
        match self {
            ConnType::WebSocket => 0,
            ConnType::WebRTC => 1,
        }
    }

    /// All connection types
    const ALL: [ConnType; 2] = [ConnType::WebSocket, ConnType::WebRTC];
}

impl From<&WireHandle> for ConnType {
    fn from(conn: &WireHandle) -> Self {
        match conn {
            WireHandle::WebSocket(_) => ConnType::WebSocket,
            WireHandle::WebRTC(_) => ConnType::WebRTC,
        }
    }
}

/// Set of ready connections
pub type ReadySet = HashSet<ConnType>;

/// Retry configuration
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 10000,
            multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Create ExponentialBackoff from this config
    pub fn create_backoff(&self) -> ExponentialBackoff {
        ExponentialBackoff::with_multiplier(
            Duration::from_millis(self.initial_delay_ms),
            Duration::from_millis(self.max_delay_ms),
            Some(self.max_attempts),
            self.multiplier,
        )
    }
}

/// WirePool - Wire connection pool manager
///
/// # Responsibilities
/// - Saturated concurrent connections (simultaneously attempt WebRTC + WebSocket)
/// - Automatic retry with exponential backoff
/// - Broadcast connection status (via watch channel, zero-polling)
/// - Keep all successful connections (no priority-based replacement)
///
/// # Design Highlights
/// - **Event-driven**: Use watch channels to notify status changes
/// - **Zero-polling**: Callers use `await ready_rx.changed()` to wait for connection readiness
/// - **Array optimization**: Use fixed-size array instead of HashMap
pub struct WirePool {
    /// Connection status (array optimization: WebSocket=0, WebRTC=1)
    connections: Arc<RwLock<[Option<WireStatus>; 2]>>,

    /// Ready connection set (broadcast)
    ready_tx: watch::Sender<ReadySet>,
    ready_rx: watch::Receiver<ReadySet>,

    /// Pending connection count
    pending: Arc<AtomicU8>,

    /// Retry configuration
    retry_config: RetryConfig,

    /// Closed flag (used to terminate background tasks)
    closed: Arc<AtomicBool>,
}

impl WirePool {
    /// Create new wire connection pool
    pub fn new(retry_config: RetryConfig) -> Self {
        let (tx, rx) = watch::channel(HashSet::new());

        Self {
            connections: Arc::new(RwLock::new([None, None])),
            ready_tx: tx,
            ready_rx: rx,
            pending: Arc::new(AtomicU8::new(0)),
            retry_config,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Add connection and start connection task in background
    ///
    /// Non-blocking, returns immediately and attempts connection concurrently in background
    ///
    /// # Behavior
    /// - **Unconditionally starts**: Always starts connection attempt, even if a connection already exists
    /// - Use `add_connection_smart()` if you want to skip already-ready connections
    pub fn add_connection(&self, connection: WireHandle) {
        let connections = Arc::clone(&self.connections);
        let ready_tx = self.ready_tx.clone();
        let pending = Arc::clone(&self.pending);
        let retry_config = self.retry_config;
        let closed = Arc::clone(&self.closed);

        let conn_type = ConnType::from(&connection);

        tokio::spawn(async move {
            // Initialize status
            {
                let mut conns = connections.write().await;
                conns[conn_type.as_index()] = Some(WireStatus::Connecting);
            }

            // Create exponential backoff iterator
            let backoff = retry_config.create_backoff();

            // Retry loop using ExponentialBackoff iterator
            for (attempt, delay) in backoff.enumerate() {
                // Check if pool has been closed
                if closed.load(Ordering::Relaxed) {
                    tracing::debug!(
                        "🛑 [{:?}] Connection task terminated (pool closed)",
                        conn_type
                    );
                    return;
                }

                // Wait for delay (first attempt has 0 delay built into iterator)
                if attempt > 0 {
                    tracing::debug!(
                        "⏱️ [{:?}] Waiting {:?} before retry {}",
                        conn_type,
                        delay,
                        attempt + 1
                    );
                    tokio::time::sleep(delay).await;

                    // Check again after sleep
                    if closed.load(Ordering::Relaxed) {
                        tracing::debug!(
                            "🛑 [{:?}] Connection task terminated (pool closed)",
                            conn_type
                        );
                        return;
                    }
                }

                pending.fetch_add(1, Ordering::Relaxed);

                tracing::debug!(
                    "🔄 [{:?}] Connecting (attempt {}/{})",
                    conn_type,
                    attempt + 1,
                    retry_config.max_attempts
                );

                let result = connection.connect().await;
                pending.fetch_sub(1, Ordering::Relaxed);

                match result {
                    Ok(_) => {
                        tracing::info!(
                            "✅ [{:?}] Connection established on attempt {}",
                            conn_type,
                            attempt + 1
                        );

                        // Update status to Ready
                        {
                            let mut conns = connections.write().await;
                            conns[conn_type.as_index()] =
                                Some(WireStatus::Ready(connection.clone()));
                        }

                        // Broadcast new ready connection set (keep all connections, no replacement)
                        Self::broadcast_ready_connections(&connections, &ready_tx).await;

                        return; // Success, exit
                    }
                    Err(e) => {
                        tracing::warn!(
                            "❌ [{:?}] Connection failed on attempt {}: {}",
                            conn_type,
                            attempt + 1,
                            e
                        );
                    }
                }
            }

            // All retries failed
            tracing::error!(
                "💀 [{:?}] All {} retries exhausted",
                conn_type,
                retry_config.max_attempts
            );

            let mut conns = connections.write().await;
            conns[conn_type.as_index()] = Some(WireStatus::Failed);

            // Check if all connections failed
            let remaining = pending.load(Ordering::Relaxed);
            if remaining == 0 {
                let all_failed = conns
                    .iter()
                    .all(|s| matches!(s, Some(WireStatus::Failed) | None));

                if all_failed {
                    tracing::error!("💀💀 All connections failed");
                }
            }
        });
    }

    /// Add connection smartly - skip if already Ready or Connecting
    ///
    /// # Behavior
    /// - **Ready**: Skip (reuse existing connection)
    /// - **Connecting**: Skip (avoid duplicate retry)
    /// - **None/Failed**: Start connection attempt
    ///
    /// # Use Case
    /// Perfect for reconnection scenarios where you want to retry failed connections
    /// without disrupting working ones.
    pub async fn add_connection_smart(&self, connection: WireHandle) {
        let conn_type = ConnType::from(&connection);

        // Check current status
        let should_add = {
            let conns = self.connections.read().await;
            match &conns[conn_type.as_index()] {
                Some(WireStatus::Ready(_)) => {
                    tracing::debug!("⏭️ [{:?}] Skipping - already Ready", conn_type);
                    false
                }
                Some(WireStatus::Connecting) => {
                    tracing::debug!("⏭️ [{:?}] Skipping - already Connecting", conn_type);
                    false
                }
                Some(WireStatus::Failed) | None => {
                    tracing::info!(
                        "🔄 [{:?}] Starting connection (was {:?})",
                        conn_type,
                        conns[conn_type.as_index()]
                    );
                    true
                }
            }
        };

        if should_add {
            self.add_connection(connection);
        }
    }

    /// Broadcast current ready connections
    async fn broadcast_ready_connections(
        connections: &Arc<RwLock<[Option<WireStatus>; 2]>>,
        ready_tx: &watch::Sender<ReadySet>,
    ) {
        let conns = connections.read().await;

        // Collect all ready connections
        let mut ready_set: ReadySet = HashSet::new();

        for conn_type in ConnType::ALL {
            if let Some(WireStatus::Ready(_)) = &conns[conn_type.as_index()] {
                ready_set.insert(conn_type);
            }
        }

        // Broadcast ready set
        let _ = ready_tx.send(ready_set);
    }

    /// Watch for connection status changes
    pub fn watch_ready(&self) -> watch::Receiver<ReadySet> {
        self.ready_rx.clone()
    }

    /// Get current ready connection set
    pub fn get_ready(&self) -> ReadySet {
        self.ready_rx.borrow().clone()
    }

    /// Get connection of specified type
    pub async fn get_connection(&self, conn_type: ConnType) -> Option<WireHandle> {
        let conns = self.connections.read().await;

        match &conns[conn_type.as_index()] {
            Some(WireStatus::Ready(conn)) => Some(conn.clone()),
            _ => None,
        }
    }

    /// Wait for any connection to become ready
    pub async fn wait_for_any(&self) -> NetworkResult<()> {
        let mut rx = self.ready_rx.clone();

        rx.wait_for(|ready_set| !ready_set.is_empty())
            .await
            .map_err(|_| {
                super::error::NetworkError::ChannelClosed("watch channel closed".to_string())
            })?;

        Ok(())
    }

    /// Mark a connection as closed/failed
    ///
    /// Called by upper layers (DestTransport) when closing connections.
    /// This replaces the per-connection event listener pattern.
    pub async fn mark_connection_closed(&self, conn_type: ConnType) {
        {
            let mut conns = self.connections.write().await;
            conns[conn_type.as_index()] = Some(WireStatus::Failed);
        }

        // Update ready set
        Self::broadcast_ready_connections(&self.connections, &self.ready_tx).await;

        tracing::debug!("🔌 Marked {:?} connection as closed", conn_type);
    }

    /// Close all connections in the pool
    ///
    /// Called by DestTransport.close() to clean up all connections.
    /// This also terminates all background connection tasks.
    pub async fn close_all(&self) {
        // 1. Set closed flag to terminate background tasks
        self.closed.store(true, Ordering::Relaxed);

        // 2. Clear all connection status
        let mut conns = self.connections.write().await;
        *conns = [None, None];

        // 3. Broadcast empty ready set
        let _ = self.ready_tx.send(HashSet::new());

        tracing::debug!("🔌 Closed all connections in pool (background tasks will terminate)");
    }

    /// Check if pool is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}
