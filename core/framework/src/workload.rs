//! Workload trait - Executable actor workload

use actr_protocol::{ActorResult, ActrId};
use async_trait::async_trait;

use crate::{Context, MessageDispatcher};

/// Workload - Executable Actor workload
///
/// Represents a complete Actor instance, including:
/// - Associated dispatcher type (Dispatcher)
/// - Lifecycle hooks (on_start, on_ready, on_stop, on_error)
/// - Network awareness hooks (signaling, WebSocket, WebRTC)
///
/// # Design Characteristics
///
/// - **Bidirectional association**: `Workload::Dispatcher` and `MessageDispatcher::Workload` reference each other
/// - **Default implementations**: All hooks have default no-op implementations, users can optionally override
/// - **Auto-implementation**: Implemented for wrapper types by code generator
///
/// # Hook Categories (v1)
///
/// Hooks are organized into three categories. Override only the hooks you need.
///
/// ## Lifecycle
/// - `on_start` — ActrNode started
/// - `on_ready` — signaling connected, registration complete
/// - `on_stop` — shutdown signal received
/// - `on_error` — runtime error caught
///
/// ## Signaling
/// - `on_signaling_connect_start` — begin connecting to signaling server
/// - `on_signaling_connected` — signaling connected, Actor is online
/// - `on_signaling_disconnected` — signaling disconnected, Actor is offline
///
/// ## Transport (WebSocket C/S)
/// - `on_websocket_connect_start` — begin establishing WebSocket connection to peer
/// - `on_websocket_connected` — WebSocket connection established
/// - `on_websocket_disconnected` — WebSocket connection lost
///
/// ## Transport (WebRTC P2P)
/// - `on_webrtc_connect_start` — begin establishing WebRTC P2P connection to peer
/// - `on_webrtc_connected` — WebRTC P2P connection established (with relay info)
/// - `on_webrtc_disconnected` — WebRTC P2P connection lost
///
/// # Code Generation Example
///
/// ```rust,ignore
/// // User-implemented Handler
/// pub struct MyEchoService { /* ... */ }
///
/// impl EchoServiceHandler for MyEchoService {
///     async fn echo<C: Context>(
///         &self,
///         req: EchoRequest,
///         ctx: &C,
///     ) -> ActorResult<EchoResponse> {
///         // Business logic
///         Ok(EchoResponse { reply: format!("Echo: {}", req.message) })
///     }
/// }
///
/// // Code-generated Workload wrapper
/// pub struct EchoServiceWorkload<T: EchoServiceHandler>(pub T);
///
/// impl<T: EchoServiceHandler> Workload for EchoServiceWorkload<T> {
///     type Dispatcher = EchoServiceRouter<T>;
/// }
/// ```
#[async_trait]
pub trait Workload: Send + Sync + 'static {
    /// Associated dispatcher type
    type Dispatcher: MessageDispatcher<Workload = Self>;

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // lifecycle
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Called when ActrNode has started.
    ///
    /// Use this to initialize business resources, start timers, etc.
    async fn on_start<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }

    /// Called when signaling is connected and registration is complete.
    ///
    /// The Actor is now discoverable and ready to serve.
    /// Typical use: set `is_ready` flag, begin accepting requests.
    async fn on_ready<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }

    /// Called when ActrNode receives a shutdown signal.
    ///
    /// Use this to release business resources and persist state.
    async fn on_stop<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }

    /// Called when the framework catches a runtime error.
    ///
    /// The `error` parameter is a string description of the error.
    /// Use this for alerting, logging, or graceful degradation.
    async fn on_error<C: Context>(&self, _ctx: &C, _error: String) -> ActorResult<()> {
        Ok(())
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // signaling
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Called when signaling connection attempt begins.
    ///
    /// `ctx` is `None` during the initial connection (before registration),
    /// and `Some` for all subsequent reconnections.
    async fn on_signaling_connect_start<C: Context>(&self, _ctx: Option<&C>) -> ActorResult<()> {
        Ok(())
    }

    /// Called when signaling connection is established. Actor is online.
    ///
    /// `ctx` is `None` during the initial connection (before registration),
    /// and `Some` for all subsequent reconnections.
    async fn on_signaling_connected<C: Context>(&self, _ctx: Option<&C>) -> ActorResult<()> {
        Ok(())
    }

    /// Called when signaling connection is lost. Actor is offline.
    async fn on_signaling_disconnected<C: Context>(&self, _ctx: &C) -> ActorResult<()> {
        Ok(())
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // transport — WebSocket C/S
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Called when a WebSocket connection attempt to a peer begins.
    async fn on_websocket_connect_start<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
    ) -> ActorResult<()> {
        Ok(())
    }

    /// Called when a WebSocket connection to a peer is established.
    async fn on_websocket_connected<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
    ) -> ActorResult<()> {
        Ok(())
    }

    /// Called when a WebSocket connection to a peer is lost.
    async fn on_websocket_disconnected<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
    ) -> ActorResult<()> {
        Ok(())
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // transport — WebRTC P2P
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    /// Called when a WebRTC P2P connection attempt to a peer begins.
    async fn on_webrtc_connect_start<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
    ) -> ActorResult<()> {
        Ok(())
    }

    /// Called when a WebRTC P2P connection to a peer is established.
    ///
    /// `relayed` indicates whether the connection goes through a TURN relay
    /// (`true`) or is a direct peer-to-peer connection (`false`).
    async fn on_webrtc_connected<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
        _relayed: bool,
    ) -> ActorResult<()> {
        Ok(())
    }

    /// Called when a WebRTC P2P connection to a peer is lost.
    async fn on_webrtc_disconnected<C: Context>(
        &self,
        _ctx: &C,
        _dest: &ActrId,
    ) -> ActorResult<()> {
        Ok(())
    }
}
