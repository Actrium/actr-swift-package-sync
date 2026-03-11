//! OutprocTransportManager - Cross-process transport manager
//!
//! Manages transport layer for multiple Dests, providing unified send/recv interface
//!
//! # Naming Convention
//! - **OutprocTransportManager**: Manages cross-process communication (WebRTC, WebSocket)
//! - **InprocTransportManager**: Manages intra-process communication (mpsc channels)
//!
//! These two form a symmetric design, handling different transport scenarios

use super::Dest; // Re-exported from actr-framework
use super::dest_transport::DestTransport;
use super::error::{NetworkError, NetworkResult};
use super::wire_handle::WireHandle;
use actr_protocol::{ActrId, PayloadType};
use async_trait::async_trait;
use either::Either;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_util::sync::CancellationToken;

/// Wire builder trait: asynchronously creates Wire components based on Dest
///
/// Implement this trait to customize Wire layer component creation logic (e.g., WebRTC, WebSocket)
#[async_trait]
pub trait WireBuilder: Send + Sync {
    /// Create Wire handle list to specified Dest
    ///
    /// # Arguments
    /// - `dest`: Target destination
    ///
    /// # Returns
    /// - Wire handle list (may contain multiple types: WebSocket, WebRTC, etc.)
    async fn create_connections(&self, dest: &Dest) -> NetworkResult<Vec<WireHandle>>;

    /// Create Wire handle list with cancellation support
    ///
    /// # Arguments
    /// - `dest`: Target destination
    /// - `cancel_token`: Optional cancellation token to terminate the operation
    ///
    /// # Returns
    /// - Wire handle list (may contain multiple types: WebSocket, WebRTC, etc.)
    /// - Returns error if cancelled
    ///
    /// Default implementation ignores the cancel token and calls `create_connections`.
    async fn create_connections_with_cancel(
        &self,
        dest: &Dest,
        cancel_token: Option<CancellationToken>,
    ) -> NetworkResult<Vec<WireHandle>> {
        // Check if already cancelled
        if let Some(ref token) = cancel_token {
            if token.is_cancelled() {
                return Err(NetworkError::ConnectionClosed(
                    "Connection creation cancelled".to_string(),
                ));
            }
        }

        // Default: just call create_connections
        self.create_connections(dest).await
    }
}

/// Destination transport state
///
/// Uses Either to manage connection lifecycle:
/// - Left: Connecting state with shared Notify (multiple waiters)
/// - Right: Connected state with DestTransport
type DestState = Either<Arc<Notify>, Arc<DestTransport>>;

/// OutprocTransportManager - Cross-process transport manager
///
/// Responsibilities:
/// - Manage transport layer for multiple Dests (each Dest maps to one DestTransport)
/// - Create DestTransport on-demand (lazy initialization)
/// - Provide unified send/recv interface
/// - Support custom connection factories
/// - Prevent duplicate connection creation using Either state machine
///
/// # Comparison with InprocTransportManager
/// - **OutprocTransportManager**: Cross-process, uses WebRTC/WebSocket
/// - **InprocTransportManager**: Intra-process, uses mpsc channels, zero serialization
///
/// # State Machine
/// ```text
/// None → Connecting(Notify) → Connected(Transport)
///         ↓                      ↓
///      (multiple waiters)     (ready)
/// ```
pub struct OutprocTransportManager {
    /// Local Actor ID
    local_id: ActrId,

    /// Dest → DestState mapping (Either state machine)
    transports: Arc<RwLock<HashMap<Dest, DestState>>>,

    /// Wire builder (used to create Wire handles for new DestTransport)
    conn_factory: Arc<dyn WireBuilder>,

    /// Cancellation tokens for in-progress connection creation
    /// Dest → CancellationToken (for cancelling ongoing connection attempts)
    pending_tokens: Arc<Mutex<HashMap<Dest, CancellationToken>>>,

    #[allow(unused)]
    /// todo: Set of peers currently being closed (to reject new connection attempts) ,closed requests will be cleaned up in event listener
    closing_peers: Arc<RwLock<HashSet<Dest>>>,
}

impl OutprocTransportManager {
    /// Create new OutprocTransportManager
    ///
    /// # Arguments
    /// - `local_id`: Local Actor ID
    /// - `conn_factory`: Wire builder, asynchronously creates Wire handle list based on Dest
    pub fn new(local_id: ActrId, conn_factory: Arc<dyn WireBuilder>) -> Self {
        Self {
            local_id,
            transports: Arc::new(RwLock::new(HashMap::new())),
            conn_factory,
            pending_tokens: Arc::new(Mutex::new(HashMap::new())),
            closing_peers: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Check if a destination is currently being closed
    pub async fn is_closing(&self, dest: &Dest) -> bool {
        self.closing_peers.read().await.contains(dest)
    }

    /// Get or create DestTransport for specified Dest
    ///
    /// # Arguments
    /// - `dest`: Target destination
    ///
    /// # Returns
    /// - DestTransport for this Dest (Arc-shared)
    ///
    /// # State Machine
    /// Uses Either to prevent duplicate connections:
    /// 1. If Connected → return transport
    /// 2. If Connecting → wait for notify, then retry
    /// 3. If None → insert Connecting(notify), create connection outside lock
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn get_or_create_transport(&self, dest: &Dest) -> NetworkResult<Arc<DestTransport>> {
        // 0. Check if dest is being closed - fast fail
        // if self.closing_peers.read().await.contains(dest) {
        //     return Err(NetworkError::ConnectionClosed(format!(
        //         "Destination {:?} is being closed.",
        //         dest
        //     )));
        // }

        loop {
            // 1. Fast path: check current state
            let state_opt = {
                let transports = self.transports.read().await;
                transports.get(dest).cloned()
            };

            match state_opt {
                // Already connected - fast path
                Some(Either::Right(transport)) => {
                    tracing::debug!("📦 Reusing existing DestTransport: {:?}", dest);
                    return Ok(transport);
                }
                // Currently connecting - wait for completion
                Some(Either::Left(notify)) => {
                    tracing::debug!("⏳ Waiting for ongoing connection: {:?}", dest);
                    notify.notified().await;
                    // Check if cancelled during wait
                    // if self.closing_peers.read().await.contains(dest) {
                    //     return Err(NetworkError::ConnectionClosed(format!(
                    //         "Destination {:?} was closed while waiting",
                    //         dest
                    //     )));
                    // }
                    // Retry after notification
                    continue;
                }
                // Not exists - need to create
                None => {
                    // Enter slow path
                }
            }

            // 2. Slow path: try to become the creator
            let notify = {
                let mut transports = self.transports.write().await;

                // Double-check: may have been created while waiting for write lock
                match transports.get(dest) {
                    Some(Either::Right(transport)) => {
                        return Ok(Arc::clone(transport));
                    }
                    Some(Either::Left(notify)) => {
                        // Another thread is creating, wait for it
                        Arc::clone(notify)
                    }
                    None => {
                        // Check closing again before creating
                        // if self.closing_peers.read().await.contains(dest) {
                        //     return Err(NetworkError::ConnectionClosed(format!(
                        //         "Destination {:?} is being closed",
                        //         dest
                        //     )));
                        // }
                        // We are the creator, insert Connecting state
                        let notify = Arc::new(Notify::new());
                        transports.insert(dest.clone(), Either::Left(Arc::clone(&notify)));
                        tracing::debug!("🔄 Inserted Connecting state for: {:?}", dest);
                        Arc::clone(&notify)
                    }
                }
            };

            // Check if we are the creator (notify was just created)
            let is_creator = {
                let transports = self.transports.read().await;
                matches!(transports.get(dest), Some(Either::Left(n)) if Arc::ptr_eq(n, &notify))
            };

            if !is_creator {
                // Wait for the actual creator
                tracing::debug!("⏳ Another thread is creating connection: {:?}", dest);
                // notify 加超时 10秒
                match tokio::time::timeout(Duration::from_secs(10), notify.notified()).await {
                    Ok(_) => continue,
                    Err(e) => {
                        return Err(NetworkError::TimeoutError(format!(
                            "Timeout waiting for notification: {:?} {}",
                            dest, e
                        )));
                    }
                }
            }

            // 3. We are the creator - create connections OUTSIDE lock
            tracing::info!("🚀 Creating new connection for: {:?}", dest);

            // Create cancellation token for this connection attempt
            let cancel_token = CancellationToken::new();
            {
                let mut tokens = self.pending_tokens.lock().await;
                tokens.insert(dest.clone(), cancel_token.clone());
            }

            let result = async {
                let connections = self
                    .conn_factory
                    .create_connections_with_cancel(dest, Some(cancel_token.clone()))
                    .await?;

                if connections.is_empty() {
                    return Err(NetworkError::ConfigurationError(format!(
                        "Connection factory returned no connections: {dest:?}"
                    )));
                }

                tracing::info!(
                    "✨ Creating DestTransport: {:?} ({} connections)",
                    dest,
                    connections.len()
                );
                let transport = DestTransport::new(dest.clone(), connections).await?;
                Ok(Arc::new(transport))
            }
            .await;

            // 4. Clean up pending token (connection attempt finished)
            {
                let mut tokens = self.pending_tokens.lock().await;
                tokens.remove(dest);
            }

            // 5. Update state and notify waiters
            let mut transports = self.transports.write().await;

            match result {
                Ok(transport) => {
                    tracing::info!("✅ Connection established: {:?}", dest);
                    transports.insert(dest.clone(), Either::Right(Arc::clone(&transport)));
                    drop(transports);
                    self.spawn_ready_monitor(dest.clone(), Arc::clone(&transport));
                    notify.notify_waiters();
                    return Ok(transport);
                }
                Err(e) => {
                    tracing::error!("❌ Connection failed: {:?}: {}", dest, e);
                    transports.remove(dest);
                    drop(transports);
                    notify.notify_waiters();
                    return Err(e);
                }
            }
        }
    }

    /// Send message to specified Dest
    ///
    /// # Arguments
    /// - `dest`: Target destination
    /// - `payload_type`: Message type
    /// - `data`: Message data
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// mgr.send(&dest, PayloadType::RpcSignal, b"hello").await?;
    /// ```
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "OutprocTransportManager.send")
    )]
    pub async fn send(
        &self,
        dest: &Dest,
        payload_type: PayloadType,
        data: &[u8],
    ) -> NetworkResult<()> {
        tracing::debug!(
            "📤 [OutprocTransportManager] Sending to {:?}: type={:?}, size={}",
            dest,
            payload_type,
            data.len()
        );

        // Get or create DestTransport for this Dest
        let transport = self.get_or_create_transport(dest).await?;

        // Send through DestTransport
        transport.send(payload_type, data).await
    }

    /// Close DestTransport for specified Dest
    ///
    /// Called by OutprocOutGate when connection events indicate cleanup is needed.
    /// This triggers the cleanup chain: OutprocTransportManager → DestTransport → WirePool
    ///
    /// # Arguments
    /// - `dest`: Target destination
    pub async fn close_transport(&self, dest: &Dest) -> NetworkResult<()> {
        // 1. Mark as closing
        self.closing_peers.write().await.insert(dest.clone());

        // 2. Cancel in-progress connection creation
        {
            let mut tokens = self.pending_tokens.lock().await;
            if let Some(token) = tokens.remove(dest) {
                tracing::info!("🚫 Cancelling in-progress connection for {:?}", dest);
                token.cancel();
            }
        }

        // 3. Remove and close the transport
        let mut transports = self.transports.write().await;

        if let Some(state) = transports.remove(dest) {
            drop(transports); // Release lock before calling close()

            match state {
                Either::Right(transport) => {
                    tracing::info!("🔌 Closing DestTransport: {:?}", dest);
                    transport.close().await?;
                }
                Either::Left(notify) => {
                    tracing::debug!("⏸️ Removed Connecting state for: {:?}", dest);
                    // Notify waiters that connection was cancelled
                    notify.notify_waiters();
                }
            }
        }

        // 4. Remove from closing set after cleanup completes
        self.closing_peers.write().await.remove(dest);

        Ok(())
    }

    /// Close all DestTransports
    pub async fn close_all(&self) -> NetworkResult<()> {
        let mut transports = self.transports.write().await;

        tracing::info!(
            "🔌 Closing all DestTransports (count: {})",
            transports.len()
        );

        for (dest, state) in transports.drain() {
            match state {
                Either::Right(transport) => {
                    if let Err(e) = transport.close().await {
                        tracing::warn!("❌ Failed to close DestTransport {:?}: {}", dest, e);
                    }
                }
                Either::Left(_notify) => {
                    tracing::debug!("⏸️ Skipped Connecting state for: {:?}", dest);
                }
            }
        }

        Ok(())
    }

    /// Get count of currently managed Dests
    pub async fn dest_count(&self) -> usize {
        self.transports.read().await.len()
    }

    /// Get local Actor ID
    #[inline]
    pub fn local_id(&self) -> &ActrId {
        &self.local_id
    }

    /// List all connected Dests
    pub async fn list_dests(&self) -> Vec<Dest> {
        self.transports.read().await.keys().cloned().collect()
    }

    /// Check if connection to specified Dest exists
    pub async fn has_dest(&self, dest: &Dest) -> bool {
        self.transports.read().await.contains_key(dest)
    }

    /// Monitor a DestTransport ready-set and remove it when all connections are gone.
    fn spawn_ready_monitor(&self, dest: Dest, transport: Arc<DestTransport>) {
        let transports = Arc::clone(&self.transports);
        tokio::spawn(async move {
            let mut rx = transport.watch_ready();
            let mut had_ready = !rx.borrow().is_empty();

            loop {
                if rx.changed().await.is_err() {
                    break;
                }
                let ready = rx.borrow().clone();

                if ready.is_empty() && had_ready {
                    // Only remove if the same transport is still mapped.
                    let mut map = transports.write().await;
                    let matched = matches!(
                        map.get(&dest),
                        Some(Either::Right(existing)) if Arc::ptr_eq(existing, &transport)
                    );
                    if matched {
                        map.remove(&dest);
                        drop(map);

                        tracing::warn!(
                            "🧹 Removing DestTransport for {:?} after all connections closed",
                            dest
                        );
                        if let Err(e) = transport.close().await {
                            tracing::warn!("⚠️ Failed to close DestTransport {:?}: {}", dest, e);
                        }
                    }
                    break;
                }

                if !ready.is_empty() {
                    had_ready = true;
                }
            }
        });
    }

    /// Spawn health checker background task with smart reconnect
    ///
    /// Periodically checks all DestTransport health status:
    /// - If some connections failed → trigger smart reconnect (reuse working connections)
    /// - If all connections failed → remove entire DestTransport
    ///
    /// # Arguments
    /// - `interval`: Health check interval (recommended: 10-30 seconds)
    ///
    /// # Returns
    /// - JoinHandle for the background task (can be used to cancel)
    ///
    /// # Example
    /// ```rust,ignore
    /// let mgr = Arc::new(OutprocTransportManager::new(local_id, factory));
    /// let health_check_handle = mgr.spawn_health_checker(Duration::from_secs(10));
    /// ```
    pub fn spawn_health_checker(&self, interval: Duration) -> tokio::task::JoinHandle<()> {
        let transports = Arc::clone(&self.transports);
        let conn_factory = Arc::clone(&self.conn_factory);

        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                interval_timer.tick().await;

                // Collect snapshot of connected Dests first (no async under lock)
                let snapshot: Vec<(Dest, Arc<DestTransport>)> = {
                    let transports_read = transports.read().await;

                    transports_read
                        .iter()
                        .filter_map(|(dest, state)| {
                            // Only check Connected transports, skip Connecting
                            if let Either::Right(transport) = state {
                                Some((dest.clone(), Arc::clone(transport)))
                            } else {
                                None
                            }
                        })
                        .collect()
                };

                // Process each Dest outside of the lock
                for (dest_clone, transport) in snapshot {
                    let healthy = transport.has_healthy_connection().await;

                    if !healthy {
                        // All connections failed - schedule for removal
                        tracing::warn!(
                            "💀 All connections failed for {:?}, will remove",
                            dest_clone
                        );

                        // Remove entire DestTransport
                        let mut transports_write = transports.write().await;
                        if let Some(Either::Right(transport)) = transports_write.remove(&dest_clone)
                        {
                            tracing::info!(
                                "🗑️  Removing completely failed DestTransport: {:?}",
                                dest_clone
                            );
                            // Drop lock before awaiting close
                            drop(transports_write);

                            if let Err(e) = transport.close().await {
                                tracing::warn!(
                                    "❌ Failed to close DestTransport {:?}: {}",
                                    dest_clone,
                                    e
                                );
                            }
                        } else {
                            // State changed between snapshot and removal; skip safely
                            drop(transports_write);
                        }
                    } else {
                        // At least one connection is working
                        // Try to reconnect failed ones (smart reconnect)
                        tracing::debug!("🔄 Triggering smart reconnect for: {:?}", dest_clone);
                        if let Err(e) = transport
                            .retry_failed_connections(&dest_clone, conn_factory.as_ref())
                            .await
                        {
                            tracing::warn!("❌ Smart reconnect failed for {:?}: {}", dest_clone, e);
                        }
                    }
                }
            }
        })
    }
}

impl Drop for OutprocTransportManager {
    fn drop(&mut self) {
        tracing::debug!("🗑️  OutprocTransportManager dropped");
        // Note: async cleanup requires external call to close_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFactory;

    #[async_trait]
    impl WireBuilder for TestFactory {
        async fn create_connections(&self, _dest: &Dest) -> NetworkResult<Vec<WireHandle>> {
            // Test factory: returns empty list (real usage requires actual connections)
            Ok(vec![])
        }
    }

    fn create_test_factory() -> Arc<dyn WireBuilder> {
        Arc::new(TestFactory)
    }

    #[tokio::test]
    async fn test_transport_manager_creation() {
        let local_id = ActrId::default();
        let factory = create_test_factory();
        let mgr = OutprocTransportManager::new(local_id.clone(), factory);

        assert_eq!(mgr.dest_count().await, 0);
        assert_eq!(mgr.local_id(), &local_id);
    }

    #[tokio::test]
    async fn test_list_dests() {
        let local_id = ActrId::default();
        let factory = create_test_factory();
        let mgr = OutprocTransportManager::new(local_id, factory);

        let dests = mgr.list_dests().await;
        assert_eq!(dests.len(), 0);
    }

    #[tokio::test]
    async fn test_has_dest() {
        let local_id = ActrId::default();
        let factory = create_test_factory();
        let mgr = OutprocTransportManager::new(local_id, factory);

        let dest = Dest::shell();
        assert!(!mgr.has_dest(&dest).await);
    }
}
