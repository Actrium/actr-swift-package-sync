//! DestTransport - Transport layer manager for a single destination
//!
//! Manages all connections and message routing to a specific Dest (Actor or Shell).
//! Implements event-driven pattern with zero polling.

use super::Dest; // Re-exported from actr-framework
use super::error::{NetworkError, NetworkResult};
use super::route_table::PayloadTypeExt;
use super::wire_handle::WireHandle;
use super::wire_pool::{ConnType, ReadySet, RetryConfig, WirePool};
use actr_protocol::PayloadType;
use std::sync::Arc;
use tokio::sync::watch;

/// DestTransport - Transport layer manager for a single destination
///
/// Core responsibilities:
/// - Manage all connections to a specific Dest (WebSocket + WebRTC)
/// - Concurrently establish connections in background (saturated connection pattern)
/// - Event-driven wait for connection status
/// - Cache Lanes within WireHandle
/// - WirePool handles priority selection
pub struct DestTransport {
    /// Connection manager
    conn_mgr: Arc<WirePool>,
}

impl DestTransport {
    /// Create new DestTransport
    ///
    /// # Arguments
    /// - `dest`: destination
    /// - `connections`: list of pre-built connections (WebSocket/WebRTC)
    pub async fn new(dest: Dest, connections: Vec<WireHandle>) -> NetworkResult<Self> {
        let conn_mgr = Arc::new(WirePool::new(RetryConfig::default()));

        // Start connection tasks in background (concurrently)
        tracing::info!("🚀 [{:?}] Starting connection tasks...", dest);
        for conn in connections {
            conn_mgr.add_connection(conn);
        }

        Ok(Self { conn_mgr })
    }

    /// Send message
    ///
    /// Core design: event-driven waiting
    /// - If connection available, send immediately
    /// - If not, wait for connection status change (via watch channel)
    /// - WirePool already handles priority, only need to try DataLane Types in order
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "DestTransport.send")
    )]
    pub async fn send(&self, payload_type: PayloadType, data: &[u8]) -> NetworkResult<()> {
        tracing::debug!(
            "📤 Sending message: type={:?}, size={}",
            payload_type,
            data.len()
        );

        // 1. Get supported DataLane Types for this PayloadType (already prioritized)
        let lane_types = payload_type.data_lane_types();

        if lane_types.is_empty() {
            return Err(NetworkError::NoRoute(format!(
                "No route for: {payload_type:?}"
            )));
        }

        // 2. Subscribe to connection status changes
        let mut conn_watcher = self.conn_mgr.watch_ready();

        loop {
            // 3. Check currently available connections (clone to avoid borrowing across await)
            let ready_connections = {
                let ready = conn_watcher.borrow_and_update();
                tracing::trace!("🔍 Available connections: {:?}", ready);
                ready.clone()
            };

            // 4. Try each DataLane Type in priority order
            for &lane_type in lane_types {
                // Determine required connection type
                let conn_type = if lane_type.needs_webrtc() {
                    ConnType::WebRTC
                } else {
                    ConnType::WebSocket
                };

                // Check if this connection is ready
                if !ready_connections.contains(&conn_type) {
                    tracing::trace!("🔍 {:?} not ready, trying next", conn_type);
                    continue;
                }

                // Get connection and create/get Lane
                if let Some(conn) = self.conn_mgr.get_connection(conn_type).await {
                    // Use original payload_type to create DataLane
                    match conn.get_lane(payload_type).await {
                        Ok(lane) => {
                            tracing::info!(
                                "📡 [channel={:?}] {:?} ({} bytes)",
                                conn_type,
                                payload_type,
                                data.len()
                            );
                            // Convert to Bytes (zero-copy)
                            let payload = bytes::Bytes::copy_from_slice(data);
                            let result = lane.send(payload.clone()).await;

                            // If DataChannel is closed, drop cache and retry once.
                            if let Err(NetworkError::DataChannelError(msg)) = &result {
                                if msg.contains("closed") {
                                    tracing::warn!(
                                        "♻️ DataChannel closed for {:?}, invalidating lane and retrying once",
                                        payload_type
                                    );
                                    conn.invalidate_lane(payload_type).await;
                                    if let Ok(new_lane) = conn.get_lane(payload_type).await {
                                        return new_lane.send(payload).await;
                                    }
                                }
                            }

                            return result;
                        }
                        Err(e) => {
                            tracing::warn!("❌ Failed to get DataLane: {:?}: {}", lane_type, e);
                            continue;
                        }
                    }
                }
            }

            // 5. All attempts failed, wait for connection status change
            tracing::info!("⏳ Waiting for connection status...");

            if self.conn_mgr.is_closed() {
                return Err(NetworkError::ChannelClosed(
                    "connection manager closed".into(),
                ));
            }

            // Event-driven wait!
            if conn_watcher.changed().await.is_err() {
                return Err(NetworkError::ChannelClosed(
                    "connection manager closed".into(),
                ));
            }

            tracing::debug!("🔔 Connection status updated, retrying...");
        }
    }

    /// Retry failed connections (smart reconnect)
    ///
    /// # Behavior
    /// - Calls WireBuilder to create new connections
    /// - Uses `add_connection_smart()` to skip already-working connections
    /// - Perfect for reconnection after detecting connection failures
    ///
    /// # Arguments
    /// - `dest`: destination (used by WireBuilder)
    /// - `wire_builder`: factory to create new WireHandles
    pub async fn retry_failed_connections(
        &self,
        dest: &Dest,
        wire_builder: &dyn super::WireBuilder,
    ) -> NetworkResult<()> {
        tracing::info!("🔄 Retrying failed connections for: {:?}", dest);

        // Get fresh connections from builder (no cancel token for retry)
        let connections = wire_builder
            .create_connections_with_cancel(dest, None)
            .await?;

        if connections.is_empty() {
            return Err(NetworkError::ConfigurationError(
                "WireBuilder returned no connections".to_string(),
            ));
        }

        // Add each connection smartly (skip Ready/Connecting)
        for conn in connections {
            self.conn_mgr.add_connection_smart(conn).await;
        }

        Ok(())
    }

    /// Close DestTransport and release all connection resources
    pub async fn close(&self) -> NetworkResult<()> {
        tracing::info!("🔌 Closing DestTransport");

        // 1. Get all connections and close them one by one
        for conn_type in [ConnType::WebSocket, ConnType::WebRTC] {
            if let Some(conn) = self.conn_mgr.get_connection(conn_type).await {
                if let Err(e) = conn.close().await {
                    tracing::warn!("❌ Failed to close {:?} connection: {}", conn_type, e);
                } else {
                    tracing::debug!("✅ Closed {:?} connection", conn_type);
                }

                // 2. Mark connection as closed in the pool
                // This updates the ready_tx and notifies waiters
                self.conn_mgr.mark_connection_closed(conn_type).await;
            }
        }

        // 3. Clean up the entire pool
        self.conn_mgr.close_all().await;

        Ok(())
    }

    /// Check if any connection is still healthy
    ///
    /// Used by health checker to detect failed connections
    ///
    /// # Returns
    /// - `true`: at least one connection is healthy (connected)
    /// - `false`: all connections are unhealthy or no connections exist
    pub async fn has_healthy_connection(&self) -> bool {
        for conn_type in [ConnType::WebRTC, ConnType::WebSocket] {
            if let Some(conn) = self.conn_mgr.get_connection(conn_type).await {
                if conn.is_connected() {
                    return true;
                }
            }
        }
        false
    }

    /// Subscribe to ready-set changes (used for manager-side cleanup).
    pub fn watch_ready(&self) -> watch::Receiver<ReadySet> {
        self.conn_mgr.watch_ready()
    }
}
