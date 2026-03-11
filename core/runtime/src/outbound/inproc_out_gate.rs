//! InprocOutGate - Inproc transport adapter (outbound)
//!
//! # Responsibilities
//! - Wrap InprocTransportManager (zero serialization, direct RpcEnvelope passing)
//! - Used for intra-process communication (e.g., Shell ↔ Workload)
//! - Support PayloadType routing (default Reliable)

use crate::transport::InprocTransportManager;
use actr_framework::Bytes;
use actr_protocol::ActrIdExt;
use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use std::sync::Arc;

/// InprocOutGate - Inproc transport adapter (outbound)
///
/// # Features
/// - Zero serialization: directly pass `RpcEnvelope` objects
/// - Zero copy: use mpsc channel for in-process passing
/// - PayloadType routing: defaults to Reliable, can specify other types via extension methods
/// - High performance: latency < 10μs
pub struct InprocOutGate {
    transport: Arc<InprocTransportManager>,
}

impl InprocOutGate {
    /// Create new InprocOutGate
    ///
    /// # Arguments
    /// - `transport`: InprocTransportManager instance
    pub fn new(transport: Arc<InprocTransportManager>) -> Self {
        Self { transport }
    }

    /// Send request and wait for response (with specified PayloadType and identifier)
    ///
    /// # Extension Method
    /// Used for scenarios requiring non-default PayloadType
    ///
    /// # Arguments
    /// - `_target`: Target ActorId (only for logging, not needed for intra-process communication)
    /// - `payload_type`: PayloadType (Reliable, Signal, LatencyFirst, MediaTrack)
    /// - `identifier`: Optional identifier (LatencyFirst needs channel_id, MediaTrack needs track_id)
    /// - `envelope`: Message envelope
    pub async fn send_request_with_type(
        &self,
        _target: &ActrId,
        payload_type: PayloadType,
        identifier: Option<String>,
        envelope: RpcEnvelope,
    ) -> ActorResult<Bytes> {
        tracing::debug!(
            "📤 InprocOutGate::send_request_with_type to {:?} (type={:?}, id={:?})",
            _target,
            payload_type,
            identifier
        );

        self.transport
            .send_request(payload_type, identifier, envelope)
            .await
            .map_err(|e| ActrError::Unavailable(e.to_string()))
    }

    /// Send one-way message (with specified PayloadType and identifier)
    ///
    /// # Arguments
    /// - `_target`: Target ActorId (only for logging, not needed for intra-process communication)
    /// - `payload_type`: PayloadType
    /// - `identifier`: Optional identifier
    /// - `envelope`: Message envelope
    pub async fn send_message_with_type(
        &self,
        _target: &ActrId,
        payload_type: PayloadType,
        identifier: Option<String>,
        envelope: RpcEnvelope,
    ) -> ActorResult<()> {
        tracing::debug!(
            "📤 InprocOutGate::send_message_with_type to {:?} (type={:?}, id={:?})",
            _target,
            payload_type,
            identifier
        );

        self.transport
            .send_message(payload_type, identifier, envelope)
            .await
            .map_err(|e| ActrError::Unavailable(e.to_string()))
    }

    /// Send request and wait for response (defaults to Reliable)
    ///
    /// # Arguments
    /// - `target`: Target ActorId (for logging only)
    /// - `envelope`: Message envelope
    ///
    /// # Default behavior
    /// Uses PayloadType::RpcReliable with no identifier
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "InprocOutGate.send_request")
    )]
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        tracing::info!(
            "📤 InprocOutGate::send_request to {:?}, request_id={}",
            target,
            envelope.request_id
        );

        // Default to Reliable (no identifier)
        let result = self
            .transport
            .send_request(PayloadType::RpcReliable, None, envelope)
            .await
            .map_err(|e| ActrError::Unavailable(e.to_string()));

        match &result {
            Ok(_) => tracing::info!("✅ InprocOutGate::send_request completed successfully"),
            Err(e) => tracing::error!("❌ InprocOutGate::send_request failed: {:?}", e),
        }

        result
    }

    /// Send one-way message (defaults to Reliable)
    ///
    /// # Arguments
    /// - `target`: Target ActorId (for logging only)
    /// - `envelope`: Message envelope
    ///
    /// # Default behavior
    /// Uses PayloadType::RpcReliable with no identifier
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "InprocOutGate.send_message", fields(target = ?target.to_string_repr()))
    )]
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        tracing::debug!(
            "InprocOutGate::send_message to {:?}",
            target.to_string_repr()
        );

        // Default to Reliable (no identifier)
        self.transport
            .send_message(PayloadType::RpcReliable, None, envelope)
            .await
            .map_err(|e| ActrError::Unavailable(e.to_string()))
    }

    /// Send DataStream (Fast Path)
    ///
    /// # Arguments
    /// - `_target`: Target ActorId (for logging only, not needed for intra-process)
    /// - `payload_type`: PayloadType (StreamReliable or StreamLatencyFirst)
    /// - `data`: Serialized DataStream bytes
    ///
    /// # Note
    /// For inproc, DataStream is sent via LatencyFirst channel with stream_id as identifier
    pub async fn send_data_stream(
        &self,
        _target: &ActrId,
        payload_type: PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        use actr_protocol::prost::Message as ProstMessage;

        // Deserialize to get stream_id
        let stream = actr_protocol::DataStream::decode(&*data)
            .map_err(|e| ActrError::DecodeFailure(format!("Failed to decode DataStream: {e}")))?;

        tracing::debug!(
            "📤 InprocOutGate::send_data_stream stream_id={}, sequence={}",
            stream.stream_id,
            stream.sequence
        );

        // Wrap in RpcEnvelope for transport
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key: "fast_path.data_stream".to_string(),
            payload: Some(data),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 0,
        };
        // Inject tracing context
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::inject_span_context_to_rpc;
            inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);
        }

        self.transport
            .send_message(payload_type, Some(stream.stream_id), envelope)
            .await
            .map_err(|e| ActrError::Unavailable(e.to_string()))
    }
}
