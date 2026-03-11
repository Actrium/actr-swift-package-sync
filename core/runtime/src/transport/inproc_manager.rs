//! InprocTransportManager - Intra-process transport manager
//!
//! Manages mpsc channel communication between Workload and Shell
//!
//! # Usage Examples
//!
//! ## Workload Side (Subscribe to data streams)
//!
//! ```rust,ignore
//! use actr_runtime::InprocTransportManager;
//! use std::sync::Arc;
//!
//! struct MyWorkload {
//!     inproc_mgr: Arc<InprocTransportManager>,
//! }
//!
//! impl MyWorkload {
//!     pub async fn subscribe_metrics_stream(&self) -> NetworkResult<()> {
//!         // Create LatencyFirst channel
//!         let rx = self.inproc_mgr
//!             .create_latency_first_channel("metrics-stream".to_string())
//!             .await;
//!
//!         // Start receive loop
//!         tokio::spawn(async move {
//!             loop {
//!                 let mut receiver = rx.lock().await;
//!                 if let Some(envelope) = receiver.recv().await {
//!                     // Process streaming data
//!                     println!("Received: {:?}", envelope);
//!                 }
//!             }
//!         });
//!
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ## Shell Side (Send data)
//!
//! ```rust,ignore
//! // Get InprocTransportManager from ActrNode
//! if let Some(inproc_mgr) = node.inproc_mgr() {
//!     // Send to LatencyFirst channel
//!     let envelope = RpcEnvelope { /* ... */ };
//!     inproc_mgr.send_message(
//!         PayloadType::StreamLatencyFirst,
//!         Some("metrics-stream".to_string()),
//!         envelope
//!     ).await?;
//! }
//! ```

use super::{DataLane, NetworkError, NetworkResult};
use actr_framework::Bytes;
use actr_protocol::{ActrError, PayloadType, RpcEnvelope};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock, mpsc, oneshot};

/// Inproc Transport Manager - manages intra-process transport (mpsc channels)
///
/// # Design Philosophy
/// - **Workload ↔ Shell communication bridge** (not for arbitrary Actor-to-Actor communication)
/// - **Reliable is mandatory, others are created on-demand**
/// - **Dynamic multi-channel management**: HashMap<String, Channel>
/// - **Bi-directional sharing**: Shell and Workload share the same Manager
pub struct InprocTransportManager {
    // ========== Mandatory base channel ==========
    /// Reliable channel (must exist)
    reliable_tx: mpsc::Sender<RpcEnvelope>,
    reliable_rx: Arc<Mutex<mpsc::Receiver<RpcEnvelope>>>,

    // ========== Optional specialized channels ==========
    /// Signal channel (optional, lazy creation)
    signal_channel: Arc<Mutex<Option<ChannelPair>>>,

    /// LatencyFirst channels (multi-instance, indexed by channel_id)
    latency_first_channels: Arc<RwLock<HashMap<String, ChannelPair>>>,

    /// MediaTrack channels (multi-instance, indexed by track_id)
    media_track_channels: Arc<RwLock<HashMap<String, ChannelPair>>>,

    // ========== Management data ==========
    /// Lane cache (avoid repeated creation)
    lane_cache: Arc<RwLock<HashMap<LaneKey, DataLane>>>,

    /// Pending requests (request/response matching)
    /// Sender can receive either success (Bytes) or error (ProtocolError)
    pending_requests:
        Arc<RwLock<HashMap<String, oneshot::Sender<actr_protocol::ActorResult<Bytes>>>>>,
}

/// Channel pair (tx + rx)
#[derive(Clone)]
struct ChannelPair {
    tx: mpsc::Sender<RpcEnvelope>,
    rx: Arc<Mutex<mpsc::Receiver<RpcEnvelope>>>,
}

/// Lane cache key
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct LaneKey {
    payload_type: PayloadType,
    /// channel_id (LatencyFirst) or track_id (MediaTrack)
    identifier: Option<String>,
}

impl Default for InprocTransportManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InprocTransportManager {
    /// Create new instance (only creates Reliable channel, others are lazy-initialized)
    ///
    /// InprocTransportManager manages intra-process communication channels between Workload and Shell.
    /// It does not need ActorId as all communication is within a single process.
    pub fn new() -> Self {
        let (reliable_tx, reliable_rx) = mpsc::channel(1024);

        tracing::debug!("Created InprocTransportManager");
        tracing::debug!("✨ Created Reliable channel");

        Self {
            reliable_tx,
            reliable_rx: Arc::new(Mutex::new(reliable_rx)),
            signal_channel: Arc::new(Mutex::new(None)),
            latency_first_channels: Arc::new(RwLock::new(HashMap::new())),
            media_track_channels: Arc::new(RwLock::new(HashMap::new())),
            lane_cache: Arc::new(RwLock::new(HashMap::new())),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // ========== Dynamic creation APIs ==========

    /// Ensure Signal channel exists
    async fn ensure_signal_channel(&self) -> ChannelPair {
        let mut opt = self.signal_channel.lock().await;
        if opt.is_none() {
            let (tx, rx) = mpsc::channel(1024);
            *opt = Some(ChannelPair {
                tx,
                rx: Arc::new(Mutex::new(rx)),
            });
            tracing::debug!("✨ Created Signal channel");
        }
        // Safe: we just created it if it was None
        opt.as_ref()
            .expect("Signal channel must exist after ensure_signal_channel")
            .clone()
    }

    /// Create LatencyFirst channel
    pub async fn create_latency_first_channel(
        &self,
        channel_id: String,
    ) -> Arc<Mutex<mpsc::Receiver<RpcEnvelope>>> {
        let mut channels = self.latency_first_channels.write().await;

        if !channels.contains_key(&channel_id) {
            let (tx, rx) = mpsc::channel(1024);
            let pair = ChannelPair {
                tx,
                rx: Arc::new(Mutex::new(rx)),
            };
            let rx_clone = pair.rx.clone();
            channels.insert(channel_id.clone(), pair);

            tracing::debug!("✨ Created LatencyFirst channel '{}'", channel_id);
            rx_clone
        } else {
            // Safe: we just checked contains_key
            channels
                .get(&channel_id)
                .expect("LatencyFirst channel must exist after contains_key check")
                .rx
                .clone()
        }
    }

    /// Create MediaTrack channel
    pub async fn create_media_track_channel(
        &self,
        track_id: String,
    ) -> Arc<Mutex<mpsc::Receiver<RpcEnvelope>>> {
        let mut channels = self.media_track_channels.write().await;

        if !channels.contains_key(&track_id) {
            let (tx, rx) = mpsc::channel(1024);
            let pair = ChannelPair {
                tx,
                rx: Arc::new(Mutex::new(rx)),
            };
            let rx_clone = pair.rx.clone();
            channels.insert(track_id.clone(), pair);

            tracing::debug!("✨ Created MediaTrack channel '{}'", track_id);
            rx_clone
        } else {
            // Safe: we just checked contains_key
            channels
                .get(&track_id)
                .expect("MediaTrack channel must exist after contains_key check")
                .rx
                .clone()
        }
    }

    // ========== Lane retrieval APIs ==========

    /// Get Lane (with optional channel_id/track_id)
    ///
    /// # Arguments
    /// - `payload_type`: PayloadType
    /// - `identifier`:
    ///   - `None` for Reliable/Signal
    ///   - `Some(channel_id)` for LatencyFirst
    ///   - `Some(track_id)` for MediaTrack
    pub async fn get_lane(
        &self,
        payload_type: PayloadType,
        identifier: Option<String>,
    ) -> NetworkResult<DataLane> {
        let key = LaneKey {
            payload_type,
            identifier: identifier.clone(),
        };

        // 1. Check cache
        {
            let cache = self.lane_cache.read().await;
            if let Some(lane) = cache.get(&key) {
                tracing::debug!("📦 Reusing cached Inproc DataLane: {:?}", key);
                return Ok(lane.clone());
            }
        }

        // 2. Get corresponding ChannelPair
        let pair = match payload_type {
            PayloadType::RpcReliable => ChannelPair {
                tx: self.reliable_tx.clone(),
                rx: self.reliable_rx.clone(),
            },

            PayloadType::RpcSignal => self.ensure_signal_channel().await,

            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                let channel_id = identifier
                    .as_ref()
                    .ok_or_else(|| {
                        NetworkError::InvalidArgument("DataStream requires channel_id".into())
                    })?
                    .clone();

                let channels = self.latency_first_channels.read().await;
                channels
                    .get(&channel_id)
                    .ok_or_else(|| NetworkError::ChannelNotFound(channel_id))?
                    .clone()
            }

            PayloadType::MediaRtp => {
                let track_id = identifier
                    .as_ref()
                    .ok_or_else(|| {
                        NetworkError::InvalidArgument("MediaRtp requires track_id".into())
                    })?
                    .clone();

                let channels = self.media_track_channels.read().await;
                channels
                    .get(&track_id)
                    .ok_or_else(|| NetworkError::ChannelNotFound(track_id))?
                    .clone()
            }
        };

        // 3. Create DataLane
        let lane = DataLane::mpsc_shared(payload_type, pair.tx, pair.rx);

        // 4. Cache it
        self.lane_cache.write().await.insert(key, lane.clone());

        tracing::debug!(
            "✨ Created Inproc DataLane: type={:?}, identifier={:?}",
            payload_type,
            identifier
        );

        Ok(lane)
    }

    // ========== High-level APIs ==========

    /// Send request (with response waiting)
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "InprocTransportManager.send_request")
    )]
    pub async fn send_request(
        &self,
        payload_type: PayloadType,
        identifier: Option<String>,
        envelope: RpcEnvelope,
    ) -> NetworkResult<Bytes> {
        let (response_tx, response_rx) = oneshot::channel();

        // Register pending request
        let request_id = envelope.request_id.clone();
        let timeout_ms = envelope.timeout_ms;
        self.pending_requests
            .write()
            .await
            .insert(request_id, response_tx);

        // Send
        let lane = self.get_lane(payload_type, identifier).await?;
        lane.send_envelope(envelope).await?;

        // Wait for response
        let timeout_duration = Duration::from_millis(timeout_ms as u64);
        let result = tokio::time::timeout(timeout_duration, response_rx)
            .await
            .map_err(|_| NetworkError::TimeoutError(format!("Request timeout: {}ms", timeout_ms)))?
            .map_err(|_| NetworkError::ConnectionError("Response channel closed".into()))?;

        // result is ActorResult<Bytes>, convert to NetworkError if error
        result.map_err(|e| NetworkError::Other(anyhow::anyhow!("{e}")))
    }

    /// Send one-way message
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "InprocTransportManager.send_message")
    )]
    pub async fn send_message(
        &self,
        payload_type: PayloadType,
        identifier: Option<String>,
        envelope: RpcEnvelope,
    ) -> NetworkResult<()> {
        let lane = self.get_lane(payload_type, identifier).await?;
        lane.send_envelope(envelope).await
    }

    /// Receive one message (select first available from all channels)
    ///
    /// # Returns
    /// - `Some(envelope)`: received message (response matching already handled)
    /// - `None`: all channels closed
    pub async fn recv(&self) -> Option<RpcEnvelope> {
        loop {
            tokio::select! {
                biased;

                // Signal (highest priority)
                msg = Self::recv_from_channel_opt(&self.signal_channel) => {
                    if let Some(envelope) = msg {
                        if !self.try_complete_response(&envelope).await {
                            return Some(envelope);  // It's a request
                        }
                        // It's a response, already handled, continue loop
                    }
                }

                // Reliable
                msg = Self::recv_from_channel(&self.reliable_rx) => {
                    if let Some(envelope) = msg {
                        if !self.try_complete_response(&envelope).await {
                            return Some(envelope);
                        }
                    }
                }

                // TODO: LatencyFirst and MediaTrack reception
                // Need to implement receiving from all channels in HashMap
            }
        }
    }

    /// Complete a pending request with response payload
    ///
    /// # Arguments
    /// - `request_id`: The request ID to complete
    /// - `response_bytes`: Response payload
    ///
    /// # Returns
    /// - `Ok(())`: Successfully sent response to waiting sender
    /// - `Err(NetworkError)`: No pending request found with this ID
    pub async fn complete_response(
        &self,
        request_id: &str,
        response_bytes: Bytes,
    ) -> NetworkResult<()> {
        let mut pending = self.pending_requests.write().await;
        if let Some(tx) = pending.remove(request_id) {
            let _ = tx.send(Ok(response_bytes));
            tracing::debug!("✅ Completed pending request: {}", request_id);
            Ok(())
        } else {
            Err(NetworkError::InvalidArgument(format!(
                "No pending request found for id: {request_id}"
            )))
        }
    }

    /// Complete a pending request with an error
    ///
    /// # Returns
    /// - `Ok(())`: Successfully sent error to waiting sender
    /// - `Err(NetworkError)`: No pending request found with this ID
    pub async fn complete_error(&self, request_id: &str, error: ActrError) -> NetworkResult<()> {
        let mut pending = self.pending_requests.write().await;
        if let Some(tx) = pending.remove(request_id) {
            let _ = tx.send(Err(error));
            tracing::debug!("✅ Completed pending request with error: {}", request_id);
            Ok(())
        } else {
            Err(NetworkError::InvalidArgument(format!(
                "No pending request found for id: {request_id}"
            )))
        }
    }

    /// Handle response matching (returns true if it was a response)
    async fn try_complete_response(&self, envelope: &RpcEnvelope) -> bool {
        let mut pending = self.pending_requests.write().await;
        if let Some(tx) = pending.remove(&envelope.request_id) {
            // Check if response or error
            match (&envelope.payload, &envelope.error) {
                (Some(payload), None) => {
                    let _ = tx.send(Ok(payload.clone()));
                    tracing::debug!("✅ Completed pending request: {}", envelope.request_id);
                }
                (None, Some(error)) => {
                    let protocol_err = ActrError::Unavailable(format!(
                        "RPC error {}: {}",
                        error.code, error.message
                    ));
                    let _ = tx.send(Err(protocol_err));
                    tracing::debug!(
                        "✅ Completed pending request with error: {}",
                        envelope.request_id
                    );
                }
                _ => {
                    tracing::error!(
                        "❌ Invalid RpcEnvelope: both payload and error present or both absent"
                    );
                    let _ = tx.send(Err(ActrError::DecodeFailure(
                        "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
                    )));
                }
            }
            true
        } else {
            false
        }
    }

    // ========== Helper methods ==========

    async fn recv_from_channel(
        rx: &Arc<Mutex<mpsc::Receiver<RpcEnvelope>>>,
    ) -> Option<RpcEnvelope> {
        rx.lock().await.recv().await
    }

    async fn recv_from_channel_opt(opt: &Arc<Mutex<Option<ChannelPair>>>) -> Option<RpcEnvelope> {
        let rx = {
            let guard = opt.lock().await;
            guard.as_ref().map(|pair| pair.rx.clone())
        };

        if let Some(rx) = rx {
            rx.lock().await.recv().await
        } else {
            std::future::pending().await // If doesn't exist, wait forever
        }
    }
}
