//! WebRtcGate - WebRTC-based OutboundGate implementation
//!
//! Uses WebRtcCoordinator to send/receive messages, implementing cross-process RPC communication

use super::coordinator::WebRtcCoordinator;
use crate::inbound::DataStreamRegistry;
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::set_parent_from_rpc_envelope;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{self, ActrId, ActrIdExt, DataStream, PayloadType, RpcEnvelope};
use actr_protocol::{ActorResult, ActrError};
use actr_runtime_mailbox::{Mailbox, MessagePriority};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};

/// Pending requests map type: request_id → (target_actor_id, oneshot response sender)
type PendingRequestsMap =
    Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>;
#[cfg(feature = "opentelemetry")]
use tracing::Instrument as _;

/// WebRTC Gate - OutboundGate implementation
///
/// # Responsibilities
/// - Implement OutboundGate trait
/// - Send messages using WebRtcCoordinator
/// - Serialize/deserialize RpcEnvelope (Protobuf)
/// - Track pending requests and match responses (by r  equest_id)
/// - Route messages by PayloadType (RPC → Mailbox, DataStream → Registry)
///
/// # Design Principles
/// - Response reuses Request's request_id (standard RPC semantics)
/// - Use pending_requests to distinguish: exists = Response, doesn't exist = Request
/// - Gateway layer doesn't deserialize payloads, raw bytes go directly to Mailbox
/// - **IMPORTANT**: pending_requests should be shared with OutprocOutGate
pub struct WebRtcGate {
    /// Local Actor ID
    local_id: Arc<RwLock<Option<ActrId>>>,

    /// WebRTC signaling coordinator
    coordinator: Arc<WebRtcCoordinator>,

    /// Pending requests (request_id → (target_actor_id, response channel))
    /// Used to determine if received message is Response (key exists) or Request (key doesn't exist)
    /// **Shared with OutprocOutGate** to ensure correct Response routing
    /// Can send success (Ok(Bytes)) or error (Err(ProtocolError))
    pending_requests: PendingRequestsMap,

    /// DataStream registry for fast-path message routing
    data_stream_registry: Arc<DataStreamRegistry>,
}

impl WebRtcGate {
    /// Create new WebRtcGate with shared pending_requests and DataStreamRegistry
    ///
    /// # Arguments
    /// - `coordinator`: WebRtcCoordinator instance
    /// - `pending_requests`: Shared pending requests (should be same as OutprocOutGate)
    /// - `data_stream_registry`: DataStream registry for fast-path routing
    pub fn new(
        coordinator: Arc<WebRtcCoordinator>,
        pending_requests: PendingRequestsMap,
        data_stream_registry: Arc<DataStreamRegistry>,
    ) -> Self {
        Self {
            local_id: Arc::new(RwLock::new(None)),
            coordinator,
            pending_requests,
            data_stream_registry,
        }
    }

    /// Set local Actor ID
    pub async fn set_local_id(&self, actor_id: ActrId) {
        *self.local_id.write().await = Some(actor_id);
    }

    /// Handle RpcEnvelope message (Response or Request)
    ///
    /// # Arguments
    /// - `envelope`: Deserialized RpcEnvelope
    /// - `from_bytes`: Sender's ActrId bytes (for Mailbox enqueue)
    /// - `data`: Original message bytes (for Mailbox enqueue)
    /// - `payload_type`: PayloadType to determine priority
    /// - `pending_requests`: Shared pending requests map
    /// - `mailbox`: Mailbox for enqueueing requests
    ///
    /// # Behavior
    /// - If request_id exists in pending_requests: Response → wake up waiting caller
    /// - If request_id doesn't exist: Request → enqueue to Mailbox
    async fn handle_envelope(
        envelope: RpcEnvelope,
        from_bytes: Vec<u8>,
        data: Bytes,
        payload_type: PayloadType,
        pending_requests: PendingRequestsMap,
        mailbox: Arc<dyn Mailbox>,
    ) {
        // Extract and set tracing context from envelope
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::set_parent_from_rpc_envelope;
            let span = tracing::info_span!("webrtc.receive_rpc", request_id = %envelope.request_id);
            set_parent_from_rpc_envelope(&span, &envelope);
            let _guard = span.enter();
        }
        let request_id = envelope.request_id.clone();

        // Determine if Response or Request
        let mut pending = pending_requests.write().await;
        if let Some((target, response_tx)) = pending.remove(&request_id) {
            // Response - Wake up waiting caller (bypassing disk, fast path)
            drop(pending); // Release lock
            tracing::debug!(
                "📬 Received RPC Response: request_id={}, target={}",
                request_id,
                target.to_string_repr()
            );

            // Convert envelope to result
            let result = match (envelope.payload, envelope.error) {
                (Some(payload), None) => Ok(payload),
                (None, Some(error)) => Err(ActrError::Unavailable(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                ))),
                _ => Err(ActrError::DecodeFailure(
                    "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
                )),
            };
            let _ = response_tx.send(result);
        } else {
            // Request - Enqueue to Mailbox (pass raw bytes, zero overhead)
            drop(pending); // Release lock
            tracing::debug!("📥 Received RPC Request: request_id={}", request_id);

            // Determine priority based on PayloadType
            let priority = match payload_type {
                PayloadType::RpcSignal => MessagePriority::High,
                PayloadType::RpcReliable => MessagePriority::Normal,
                _ => MessagePriority::Normal,
            };

            // Enqueue to Mailbox (from_bytes and data are original bytes, zero overhead)
            // Convert Bytes to Vec<u8> (Mailbox uses Vec)
            match mailbox.enqueue(from_bytes, data.to_vec(), priority).await {
                Ok(msg_id) => {
                    tracing::debug!(
                        "✅ RPC message enqueued to Mailbox: msg_id={}, priority={:?}",
                        msg_id,
                        priority
                    );
                }
                Err(e) => {
                    tracing::error!("❌ Mailbox enqueue failed: {:?}", e);
                }
            }
        }
    }

    /// Start message receive loop (called by ActrSystem/ActrNode)
    ///
    /// # Arguments
    /// - `mailbox`: message queue for persisting inbound requests
    ///
    /// # Architecture
    /// According to three-loop architecture design (framework-runtime-architecture.zh.md):
    /// - WebRtcGate belongs to outer loop (Transport layer)
    /// - Mailbox belongs to inner loop (state path)
    /// - Message flow: WebRTC → WebRtcGate → Mailbox/DataStreamRegistry → Scheduler → ActrNode
    ///
    /// # Message Routing Logic
    /// - Route based on PayloadType:
    ///   - RpcReliable/RpcSignal: Deserialize RpcEnvelope, check pending_requests, enqueue to Mailbox
    ///   - StreamReliable/StreamLatencyFirst: Deserialize DataStream, dispatch to DataStreamRegistry
    pub async fn start_receive_loop(&self, mailbox: Arc<dyn Mailbox>) -> ActorResult<()> {
        let coordinator = self.coordinator.clone();
        let pending_requests = self.pending_requests.clone();
        let data_stream_registry = self.data_stream_registry.clone();

        tokio::spawn(async move {
            loop {
                // Receive message from WebRtcCoordinator (now includes PayloadType)
                match coordinator.receive_message().await {
                    Ok(Some((from_bytes, data, payload_type))) => {
                        tracing::debug!(
                            "📨 WebRtcGate received message: {} bytes, PayloadType: {:?}",
                            data.len(),
                            payload_type
                        );

                        // Route based on PayloadType
                        match payload_type {
                            PayloadType::RpcReliable | PayloadType::RpcSignal => {
                                // RPC path: deserialize RpcEnvelope and route
                                match RpcEnvelope::decode(&data[..]) {
                                    Ok(envelope) => {
                                        #[cfg(feature = "opentelemetry")]
                                        let span = {
                                            let span = tracing::info_span!("webrtc.receive_rpc");
                                            set_parent_from_rpc_envelope(&span, &envelope);
                                            span
                                        };
                                        let handle_envelope_fut = Self::handle_envelope(
                                            envelope,
                                            from_bytes,
                                            data,
                                            payload_type,
                                            pending_requests.clone(),
                                            mailbox.clone(),
                                        );
                                        #[cfg(feature = "opentelemetry")]
                                        let handle_envelope_fut =
                                            handle_envelope_fut.instrument(span);

                                        handle_envelope_fut.await;
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "❌ Failed to deserialize RpcEnvelope: {:?}",
                                            e
                                        );
                                    }
                                }
                            }
                            PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                                // DataStream path: deserialize and dispatch to registry
                                match DataStream::decode(&data[..]) {
                                    Ok(chunk) => {
                                        tracing::debug!(
                                            "📦 Received DataStream: stream_id={}, seq={}, {} bytes",
                                            chunk.stream_id,
                                            chunk.sequence,
                                            chunk.payload.len()
                                        );

                                        // Decode sender ActrId
                                        match ActrId::decode(&from_bytes[..]) {
                                            Ok(sender_id) => {
                                                // Dispatch to DataStreamRegistry (async callback invocation)
                                                data_stream_registry
                                                    .dispatch(chunk, sender_id)
                                                    .await;
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "❌ Failed to decode sender ActrId: {:?}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(
                                            "❌ Failed to deserialize DataStream: {:?}",
                                            e
                                        );
                                    }
                                }
                            }
                            PayloadType::MediaRtp => {
                                tracing::warn!(
                                    "⚠️ MediaRtp received in WebRtcGate (should use RTCTrackRemote)"
                                );
                            }
                        }
                    }
                    Ok(None) => {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                    Err(e) => {
                        tracing::error!("❌ Message receive failed: {:?}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Send response (called by Mailbox handler loop)
    ///
    /// # Arguments
    /// - `target`: response target ActrId (original request sender)
    /// - `response_envelope`: response RpcEnvelope (**must reuse original request_id**)
    ///
    /// # Design Principle
    /// - Response reuses Request's request_id (caller is responsible)
    /// - Receiver matches to pending_requests by request_id and wakes up waiting caller
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn send_response(
        &self,
        target: &ActrId,
        response_envelope: RpcEnvelope,
    ) -> ActorResult<()> {
        // Serialize RpcEnvelope (Protobuf)
        let mut buf = Vec::new();
        response_envelope
            .encode(&mut buf)
            .map_err(|e| ActrError::Internal(format!("Failed to encode response: {e}")))?;

        // Send
        self.coordinator.send_message(target, &buf).await?;
        tracing::debug!(
            "📤 Sent response: request_id={}, {} bytes",
            response_envelope.request_id,
            buf.len()
        );
        Ok(())
    }
}
