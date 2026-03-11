//! ActrRef - Lightweight reference to a running Actor
//!
//! # Design Philosophy
//!
//! `ActrRef` is the primary handle for interacting with a running Actor.
//! It provides:
//!
//! - **RPC calls**: Call Actor methods (Shell → Workload)
//! - **Event subscription**: Subscribe to Actor events (Workload → Shell)
//! - **Lifecycle control**: Shutdown and wait for completion
//!
//! # Key Characteristics
//!
//! - **Cloneable**: Can be shared across tasks
//! - **Lightweight**: Contains only an `Arc` to shared state
//! - **Auto-cleanup**: Last `ActrRef` drop triggers resource cleanup
//! - **Code-gen friendly**: RPC methods will be generated and bound to this type
//!
//! # Usage
//!
//! ```rust,ignore
//! let actr = node.start().await?;
//!
//! // Clone and use in different tasks
//! let actr1 = actr.clone();
//! tokio::spawn(async move {
//!     actr1.call(SomeRequest { ... }).await?;
//! });
//!
//! // Subscribe to events
//! let mut events = actr.events();
//! while let Some(event) = events.next().await {
//!     println!("Event: {:?}", event);
//! }
//!
//! // Shutdown
//! actr.shutdown();
//! actr.wait_for_shutdown().await;
//! ```

use crate::lifecycle::ActrNode;
use crate::outbound::InprocOutGate;
use actr_framework::{Bytes, Workload};
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// ActrRef - Lightweight reference to a running Actor
///
/// This is the primary handle returned by `ActrNode::start()`.
///
/// # Code Generation Pattern
///
/// `actr-cli` code generator will generate type-safe RPC methods for `ActrRef`.
///
/// ## Proto Definition
///
/// ```protobuf
/// service EchoService {
///   rpc Echo(EchoRequest) returns (EchoResponse);
///   rpc Ping(PingRequest) returns (PingResponse);
/// }
/// ```
///
/// ## Generated Code (in `generated/echo_service_actr_ref.rs`)
///
/// ```rust,ignore
/// use actr_runtime::ActrRef;
/// use super::echo_service_actor::{EchoServiceWorkload, EchoServiceHandler};
/// use super::echo::{EchoRequest, EchoResponse, PingRequest, PingResponse};
///
/// impl<T: EchoServiceHandler> ActrRef<EchoServiceWorkload<T>> {
///     /// Call Echo RPC method
///     pub async fn echo(&self, request: EchoRequest) -> ActorResult<EchoResponse> {
///         self.call(request).await
///     }
///
///     /// Call Ping RPC method
///     pub async fn ping(&self, request: PingRequest) -> ActorResult<PingResponse> {
///         self.call(request).await
///     }
/// }
/// ```
///
/// ## Usage in Shell
///
/// ```rust,ignore
/// use generated::echo_service_actr_ref::*;  // Import ActrRef extensions
///
/// let actr = node.start().await?;
///
/// // Type-safe RPC calls (generated methods)
/// let response = actr.echo(EchoRequest {
///     message: "Hello".to_string(),
/// }).await?;
///
/// // Or use generic call() method
/// let response: EchoResponse = actr.call(EchoRequest { ... }).await?;
/// ```
///
/// # Design Rationale
///
/// **Why bind RPC methods to ActrRef?**
///
/// 1. **Type Safety**: Compiler checks request/response types
/// 2. **Auto-completion**: IDE shows available RPC methods
/// 3. **No target needed**: ActrRef already knows its target Actor
/// 4. **Symmetric to Context**: Similar to Context extension pattern
///
/// **Comparison with Context pattern:**
///
/// | Aspect | Context (in Workload) | ActrRef (in Shell) |
/// |--------|----------------------|-------------------|
/// | Caller | Workload | Shell |
/// | Target | Any Actor (needs `target` param) | This Workload (fixed) |
/// | Method | `ctx.call(target, req)` | `actr.echo(req)` |
/// | Generation | Extension trait | Concrete impl |
pub struct ActrRef<W: Workload> {
    pub(crate) shared: Arc<ActrRefShared>,
    pub(crate) node: Arc<ActrNode<W>>,
}

impl<W: Workload> Clone for ActrRef<W> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            node: Arc::clone(&self.node),
        }
    }
}

/// Shared state between all ActrRef clones
///
/// This is an internal implementation detail. When the last `ActrRef` is dropped,
/// this struct's `Drop` impl will trigger shutdown and cleanup all resources.
pub(crate) struct ActrRefShared {
    /// Actor ID
    pub(crate) actor_id: ActrId,

    /// Inproc gate for Shell → Workload RPC
    pub(crate) inproc_gate: Arc<InprocOutGate>,

    /// Shutdown signal
    pub(crate) shutdown_token: CancellationToken,

    /// Background task handles (receive loops, WebRTC coordinator, etc.)
    pub(crate) task_handles: Mutex<Vec<JoinHandle<()>>>,
}

impl<W: Workload> ActrRef<W> {
    /// Create new ActrRef from shared state
    ///
    /// This is an internal API used by `ActrNode::start()`.
    pub(crate) fn new(shared: Arc<ActrRefShared>, node: Arc<ActrNode<W>>) -> Self {
        Self { shared, node }
    }

    /// Get Actor ID
    pub fn actor_id(&self) -> &ActrId {
        &self.shared.actor_id
    }

    /// Discover remote actors of the specified type via signaling server.
    ///
    /// This method implements the full runtime compatibility negotiation workflow:
    ///
    /// 1. **Fast Path**: Check `compat.lock.toml` for cached negotiation results
    /// 2. **Ideal Path**: Read fingerprint from `Actr.lock.toml` (when available) and request exact match
    /// 3. **Negotiation**: If no exact match, server performs compatibility analysis
    /// 4. **Result**: Returns candidates with compatibility info, updates caches
    ///
    /// The fingerprint is automatically obtained from `Actr.lock.toml` when available.
    /// If it is missing and no compat.lock.toml entry exists, discovery proceeds without a fingerprint
    /// and skips compatibility negotiation.
    ///
    /// # Arguments
    /// - `target_type`: The ActrType of the target service to discover
    /// - `candidate_count`: Maximum number of candidates to return
    ///
    /// # Returns
    /// A list of compatible `ActrId` candidates.
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn discover_route_candidates(
        &self,
        target_type: &actr_protocol::ActrType,
        candidate_count: u32,
    ) -> ActorResult<Vec<ActrId>> {
        let result = self
            .node
            .discover_route_candidates(target_type, candidate_count)
            .await?;
        Ok(result.candidates)
    }

    /// Call Actor method (Shell → Workload RPC)
    ///
    /// This is a generic method used by code-generated RPC methods.
    /// Most users should use the generated methods instead.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Generic call
    /// let response: EchoResponse = actr.call(EchoRequest {
    ///     message: "Hello".to_string(),
    /// }).await?;
    ///
    /// // Generated method (preferred)
    /// let response = actr.echo(EchoRequest {
    ///     message: "Hello".to_string(),
    /// }).await?;
    /// ```
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "ActrRef.call")
    )]
    pub async fn call<R>(&self, request: R) -> ActorResult<R::Response>
    where
        R: actr_protocol::RpcRequest,
    {
        // Encode request
        let payload: Bytes = request.encode_to_vec().into();

        // Create envelope
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key: R::route_key().to_string(),
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 30000,
        };
        // Inject tracing context
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::inject_span_context_to_rpc;
            inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);
        }

        // Send request and wait for response (target is our actor_id for logging)
        let response_bytes = self
            .shared
            .inproc_gate
            .send_request(&self.shared.actor_id, envelope)
            .await?;

        // Decode response
        R::Response::decode(&*response_bytes)
            .map_err(|e| ActrError::DecodeFailure(format!("Failed to decode response: {e}")))
    }

    /// Call Actor method using route_key and request bytes (for language bindings)
    ///
    /// This is a non-generic version of `call()` that accepts route_key and raw bytes,
    /// making it suitable for language bindings (e.g., Python) that don't have access
    /// to Rust's generic `RpcRequest` trait.
    ///
    /// # Parameters
    /// - `route_key`: Route key string (e.g., "package.Service.Method")
    /// - `request_bytes`: Request protobuf bytes
    /// - `timeout_ms`: Timeout in milliseconds
    /// - `payload_type`: Payload transmission type
    ///
    /// # Returns
    /// Response protobuf bytes
    pub async fn call_raw(
        &self,
        route_key: String,
        request_bytes: Bytes,
        timeout_ms: i64,
        payload_type: PayloadType,
    ) -> ActorResult<Bytes> {
        // Create envelope
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(request_bytes),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms,
        };
        // Inject tracing context
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::inject_span_context_to_rpc;
            inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);
        }

        // Send request and wait for response
        self.shared
            .inproc_gate
            .send_request_with_type(&self.shared.actor_id, payload_type, None, envelope)
            .await
    }

    /// Send one-way message using route_key and message bytes (for language bindings)
    ///
    /// This is a non-generic version of `tell()` that accepts route_key and raw bytes,
    /// making it suitable for language bindings (e.g., Python) that don't have access
    /// to Rust's generic `RpcRequest` trait.
    ///
    /// # Parameters
    /// - `route_key`: Route key string (e.g., "package.Service.Method")
    /// - `message_bytes`: Message protobuf bytes
    /// - `payload_type`: Payload transmission type
    ///
    /// # Returns
    /// Unit (fire-and-forget, no response)
    pub async fn tell_raw(
        &self,
        route_key: String,
        message_bytes: Bytes,
        payload_type: PayloadType,
    ) -> ActorResult<()> {
        // Create envelope
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key,
            payload: Some(message_bytes),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 0, // No timeout for one-way messages
        };
        // Inject tracing context
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::inject_span_context_to_rpc;
            inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);
        }

        // Send message without waiting for response
        self.shared
            .inproc_gate
            .send_message_with_type(&self.shared.actor_id, payload_type, None, envelope)
            .await
    }

    /// Send one-way message to Actor (Shell → Workload, fire-and-forget)
    ///
    /// Unlike `call()`, this method does not wait for a response.
    /// Use this for notifications or commands that don't need acknowledgment.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Send notification without waiting for response
    /// actr.tell(LogEvent {
    ///     level: "INFO".to_string(),
    ///     message: "User logged in".to_string(),
    /// }).await?;
    ///
    /// // Generated method (if codegen supports tell)
    /// actr.log_event(LogEvent { ... }).await?;
    /// ```
    ///
    /// # Performance
    ///
    /// - **Latency**: ~10μs (in-process, zero serialization)
    /// - **No blocking**: Returns immediately after sending
    /// - **No response**: Caller won't know if message was processed
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    pub async fn tell<R>(&self, message: R) -> ActorResult<()>
    where
        R: actr_protocol::RpcRequest + ProstMessage,
    {
        // Encode message
        let payload: Bytes = message.encode_to_vec().into();

        // Create envelope (note: request_id still included for tracing)
        #[cfg_attr(not(feature = "opentelemetry"), allow(unused_mut))]
        let mut envelope = RpcEnvelope {
            route_key: R::route_key().to_string(),
            payload: Some(payload),
            error: None,
            traceparent: None,
            tracestate: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            metadata: vec![],
            timeout_ms: 0, // No timeout for one-way messages
        };
        // Inject tracing context
        #[cfg(feature = "opentelemetry")]
        {
            use crate::wire::webrtc::trace::inject_span_context_to_rpc;
            inject_span_context_to_rpc(&tracing::Span::current(), &mut envelope);
        }

        // Send message without waiting for response
        self.shared
            .inproc_gate
            .send_message(&self.shared.actor_id, envelope)
            .await
    }

    /// Trigger Actor shutdown
    ///
    /// This signals the Actor to stop, but does not wait for completion.
    /// Use `wait_for_shutdown()` to wait for cleanup to finish.
    pub fn shutdown(&self) {
        tracing::info!("🛑 Shutdown requested for Actor {:?}", self.shared.actor_id);
        self.shared.shutdown_token.cancel();
    }

    /// Wait for Actor to fully shutdown
    ///
    /// This waits for the shutdown signal to be triggered.
    /// All background tasks will be aborted when the last `ActrRef` is dropped.
    pub async fn wait_for_shutdown(&self) {
        self.shared.shutdown_token.cancelled().await;
        // Take ownership of the current handles so we can await them as Futures.
        let mut guard = self.shared.task_handles.lock().await;
        let handles = std::mem::take(&mut *guard);
        drop(guard);
        tracing::debug!("Waiting for tasks to complete: {:?}", handles.len());
        // All tasks have been asked to shut down; wait for them with a timeout,
        // and abort any that don't finish in time to avoid leaking background work.
        for handle in handles {
            let sleep = tokio::time::sleep(Duration::from_secs(5));
            tokio::pin!(handle);
            tokio::pin!(sleep);

            tokio::select! {
                res = &mut handle => {
                    match res {
                        Ok(_) => {
                            tracing::debug!("Task completed");
                        }
                        Err(e) => {
                            tracing::error!("Task failed: {:?}", e);
                        }
                    }
                }
                _ = sleep => {
                    tracing::warn!("Task timed out after 5s, aborting");
                    handle.abort();
                }
            }
        }
    }

    /// Check if Actor is shutting down
    pub fn is_shutting_down(&self) -> bool {
        self.shared.shutdown_token.is_cancelled()
    }

    ///
    /// This consumes the `ActrRef` and waits for signal (Ctrl+C / SIGTERM) , then triggers shutdown.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let actr = node.start().await?;
    /// actr.wait_for_ctrl_c_and_shutdown().await?;
    /// ```
    pub async fn wait_for_ctrl_c_and_shutdown(self) -> ActorResult<()> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};

            let mut sigint = signal(SignalKind::interrupt()).map_err(|e| {
                ActrError::Unavailable(format!("Signal handler error (SIGINT): {e}"))
            })?;
            let mut sigterm = signal(SignalKind::terminate()).map_err(|e| {
                ActrError::Unavailable(format!("Signal handler error (SIGTERM): {e}"))
            })?;

            tokio::select! {
                _ = sigint.recv() => {
                    tracing::info!("📡 Received SIGINT (Ctrl+C) signal");
                }
                _ = sigterm.recv() => {
                    tracing::info!("📡 Received SIGTERM signal");
                }
            }
        }

        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c()
                .await
                .map_err(|e| ActrError::Unavailable(format!("Ctrl+C signal error: {e}")))?;

            tracing::info!("📡 Received Ctrl+C signal");
        }

        // Call on_stop lifecycle hook before shutdown
        if let (Some(factory), Some(actor_id), Some(credential_state)) = (
            self.node.context_factory.as_ref(),
            self.node.actor_id.as_ref(),
            self.node.credential_state.as_ref(),
        ) {
            let ctx = factory.create(
                actor_id,
                None,
                "shutdown",
                &credential_state.credential().await,
            );
            tracing::info!("🪝 Calling lifecycle hook: on_stop");
            if let Err(e) = self.node.workload.on_stop(&ctx).await {
                tracing::warn!("⚠️ on_stop hook error: {}", e);
            }
            tracing::info!("✅ Lifecycle hook on_stop completed");
        }

        self.shutdown();
        self.wait_for_shutdown().await;

        Ok(())
    }
}

impl Drop for ActrRefShared {
    fn drop(&mut self) {
        tracing::info!(
            "🧹 ActrRefShared dropping - cleaning up Actor {:?}",
            self.actor_id
        );

        // Cancel shutdown token
        self.shutdown_token.cancel();

        // Abort all background tasks (best-effort)
        if let Ok(mut handles) = self.task_handles.try_lock() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        } else {
            tracing::warn!(
                "⚠️ Failed to lock task_handles mutex during Drop; some tasks may still be running"
            );
        }

        tracing::debug!(
            "✅ All background tasks aborted for Actor {:?}",
            self.actor_id
        );
    }
}
