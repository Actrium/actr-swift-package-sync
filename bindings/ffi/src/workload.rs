//! Dynamic Workload implementation for callback interfaces

use crate::ActrResult;
use crate::context::ContextBridge;
use actr_framework::{Bytes, Context, MessageDispatcher, Workload};
use actr_protocol::{ActorResult, RpcEnvelope};
use async_trait::async_trait;
use std::sync::Arc;

/// RPC Envelope exposed to FFI
///
/// Contains the route key, payload, and request ID for an RPC message.
#[derive(uniffi::Record, Clone)]
pub struct RpcEnvelopeBridge {
    /// Route key for the RPC method (e.g., "echo.EchoService.Echo")
    pub route_key: String,
    /// Request payload bytes (protobuf encoded)
    pub payload: Vec<u8>,
    /// Request ID for correlation
    pub request_id: String,
}

impl From<RpcEnvelope> for RpcEnvelopeBridge {
    fn from(envelope: RpcEnvelope) -> Self {
        Self {
            route_key: envelope.route_key,
            payload: envelope.payload.map(|p| p.to_vec()).unwrap_or_default(),
            request_id: envelope.request_id,
        }
    }
}

#[uniffi::export(callback_interface)]
#[async_trait::async_trait]
pub trait WorkloadBridge: Send + Sync + 'static {
    /// Lifecycle hook called when the workload starts
    async fn on_start(&self, ctx: Arc<ContextBridge>) -> ActrResult<()>;

    /// Lifecycle hook called when the workload stops
    async fn on_stop(&self, ctx: Arc<ContextBridge>) -> ActrResult<()>;

    /// Dispatch an incoming RPC message
    ///
    /// This method is called when the workload receives an RPC message from
    /// the Shell (local application) side. The user **must** implement the
    /// message handling logic here and return the response bytes.
    ///
    /// This is similar to `MessageDispatcher::dispatch` in Rust - you receive
    /// the request, process it (e.g., forward to a remote server), and return
    /// the response.
    ///
    /// # Arguments
    /// - `ctx`: Context for making RPC calls to remote actors
    /// - `envelope`: The incoming RPC envelope containing route_key, payload, and request_id
    ///
    /// # Returns
    /// - Response bytes (protobuf encoded)
    ///
    async fn dispatch(
        &self,
        ctx: Arc<ContextBridge>,
        envelope: RpcEnvelopeBridge,
    ) -> ActrResult<Vec<u8>>;
}

/// Dynamic workload that wraps a callback interface
#[derive(uniffi::Object)]
pub struct DynamicWorkload {
    bridge: Arc<dyn WorkloadBridge>,
}

impl DynamicWorkload {
    pub fn new(bridge: Arc<dyn WorkloadBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl Workload for DynamicWorkload {
    type Dispatcher = DynamicDispatcher;

    async fn on_start<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        self.bridge
            .on_start(ctx_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }

    async fn on_stop<C: Context>(&self, ctx: &C) -> ActorResult<()> {
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;
        self.bridge
            .on_stop(ctx_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)
    }
}

/// Dynamic dispatcher that routes messages to the callback interface
///
/// This dispatcher simply delegates to the user's `dispatch` implementation.
/// All message handling logic (discovering servers, forwarding requests, etc.)
/// must be implemented by the user in their `WorkloadBridge::dispatch` method.
pub struct DynamicDispatcher;

#[async_trait]
impl MessageDispatcher for DynamicDispatcher {
    type Workload = DynamicWorkload;

    async fn dispatch<C: Context>(
        workload: &Self::Workload,
        envelope: RpcEnvelope,
        ctx: &C,
    ) -> ActorResult<Bytes> {
        // Create context bridge for the callback
        let ctx_bridge =
            ContextBridge::try_from_context(ctx).map_err(actr_protocol::ActrError::from)?;

        // Convert envelope to bridge type
        let envelope_bridge: RpcEnvelopeBridge = envelope.into();

        // Call user's dispatch method - user must handle and return response
        let response = workload
            .bridge
            .dispatch(ctx_bridge, envelope_bridge)
            .await
            .map_err(actr_protocol::ActrError::from)?;

        Ok(Bytes::from(response))
    }
}
