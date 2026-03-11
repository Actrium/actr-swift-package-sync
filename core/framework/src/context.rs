//! Context trait - Execution context interface for actors

use actr_protocol::{ActorResult, ActrId, ActrType, DataStream, PayloadType};
use async_trait::async_trait;
use futures_util::future::BoxFuture;

/// Context - Actor execution context interface
///
/// Defines the complete interface for Actor interaction with the system, including:
/// - Context data access (self_id, request_id, etc.)
/// - Communication capabilities (call, tell)
///
/// # Design Principles
///
/// - **Interface only**: Framework does not provide implementation, runtime implements
/// - **Generic parameter**: User code uses `<C: Context>` instead of `&dyn Context`
/// - **Zero virtual calls**: Static dispatch via generic monomorphization
///
/// # Implementation Requirements
///
/// Runtime must implement this trait and use enum dispatch or other
/// zero-overhead mechanisms internally (avoiding virtual function calls).
///
/// # Example
///
/// ```rust,ignore
/// async fn my_handler<C: Context>(ctx: &C) {
///     // Access data
///     let id = ctx.self_id();
///
///     // Type-safe call
///     let response = ctx.call(&target, request).await?;
/// }
/// ```
#[async_trait]
pub trait Context: Send + Sync + Clone + 'static {
    // ========== Data Access Methods ==========

    /// Get the current Actor's ID
    fn self_id(&self) -> &ActrId;

    /// Get the caller's Actor ID
    ///
    /// - `Some(caller_id)`: Called by another Actor
    /// - `None`: System internal call (e.g., lifecycle hooks)
    fn caller_id(&self) -> Option<&ActrId>;

    /// Get the unique request ID
    ///
    /// A new request_id is generated for each RPC call, used to match requests and responses.
    fn request_id(&self) -> &str;

    // ========== Communication Methods ==========

    /// Send a type-safe RPC request and wait for response
    ///
    /// This is the primary way to call other Actors, providing full type safety guarantees.
    ///
    /// # Type Inference
    ///
    /// Response type is automatically inferred from `R::Response`, no manual annotation needed:
    ///
    /// ```rust,ignore
    /// let request = EchoRequest { message: "hello".to_string() };
    /// let response: EchoResponse = ctx.call(&target, request).await?;
    /// //              ^^^^^^^^^^^^ Inferred from EchoRequest::Response
    /// ```
    ///
    /// # Error Handling
    ///
    /// - `ProtocolError::TransportError`: Network transport failure
    /// - `ProtocolError::Actr(DecodeFailure)`: Response decode failure
    /// - `ProtocolError::Actr(UnknownRoute)`: Route does not exist
    /// - Errors returned by remote Actor's business logic
    ///
    /// # Parameters
    ///
    /// - `target`: Target destination (`Dest::Shell` for local, `Dest::Actor(id)` for remote)
    /// - `request`: Request message implementing `RpcRequest` trait
    ///
    /// # Returns
    ///
    /// Returns response message of type `R::Response`
    async fn call<R: actr_protocol::RpcRequest>(
        &self,
        target: &crate::Dest,
        request: R,
    ) -> ActorResult<R::Response>;

    /// Send a type-safe one-way message (no response expected)
    ///
    /// Used for sending notifications, events, etc. that do not require a response.
    ///
    /// # Semantics
    ///
    /// - **Fire-and-forget**: Does not wait for response after sending
    /// - **No delivery guarantee**: Message may be lost if target is unreachable
    /// - **Low latency**: Does not block waiting for response
    ///
    /// # Parameters
    ///
    /// - `target`: Target destination (`Dest::Shell` for local, `Dest::Actor(id)` for remote)
    /// - `message`: Message implementing `RpcRequest` trait
    async fn tell<R: actr_protocol::RpcRequest>(
        &self,
        target: &crate::Dest,
        message: R,
    ) -> ActorResult<()>;

    // ========== Fast Path: DataStream Methods ==========

    /// Register a DataStream callback for a specific stream
    ///
    /// When a DataStream with matching stream_id arrives, the registered callback will be invoked.
    /// Callbacks are executed concurrently and do not block other streams.
    ///
    /// # Parameters
    ///
    /// - `stream_id`: Stream identifier (must be globally unique)
    /// - `callback`: Handler function that receives (DataStream, sender ActrId)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.register_stream("log-stream", |chunk, sender| {
    ///     Box::pin(async move {
    ///         println!("Received chunk {} from {:?}", chunk.sequence, sender);
    ///         Ok(())
    ///     })
    /// }).await?;
    /// ```
    async fn register_stream<F>(&self, stream_id: String, callback: F) -> ActorResult<()>
    where
        F: Fn(DataStream, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static;

    /// Unregister a DataStream callback
    ///
    /// # Parameters
    ///
    /// - `stream_id`: Stream identifier to unregister
    async fn unregister_stream(&self, stream_id: &str) -> ActorResult<()>;

    /// Send a DataStream to a destination with explicit lane selection.
    ///
    /// Use [`PayloadType::StreamReliable`] for ordered reliable delivery (default) or
    /// [`PayloadType::StreamLatencyFirst`] for low-latency partial-reliable delivery.
    ///
    /// # Parameters
    ///
    /// - `target`: Target destination
    /// - `chunk`: DataStream to send
    /// - `payload_type`: Lane selection (`StreamReliable` or `StreamLatencyFirst`)
    async fn send_data_stream(
        &self,
        target: &crate::Dest,
        chunk: DataStream,
        payload_type: PayloadType,
    ) -> ActorResult<()>;

    /// Discover a remote Actor of the specified type via the signaling server.
    ///
    /// Returns a route candidate or an error if none are available. Concrete
    /// selection strategy is decided by the Context implementation.
    async fn discover_route_candidate(&self, target_type: &ActrType) -> ActorResult<ActrId>;

    /// Send a raw RPC request (untyped bytes) and wait for response
    ///
    /// This is a lower-level method for dynamic dispatch scenarios where the
    /// request/response types are not known at compile time (e.g., FFI bindings).
    ///
    /// # Parameters
    ///
    /// - `target`: Target Actor ID
    /// - `route_key`: Route key (e.g., "echo.EchoService/Echo")
    /// - `payload`: Raw request payload bytes
    ///
    /// # Returns
    ///
    /// Returns raw response payload bytes
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // For FFI or dynamic dispatch scenarios
    /// let response = ctx.call_raw(
    ///     &target_id,
    ///     "echo.EchoService/Echo",
    ///     request_bytes.into(),
    /// ).await?;
    /// ```
    async fn call_raw(
        &self,
        target: &ActrId,
        route_key: &str,
        payload: bytes::Bytes,
    ) -> ActorResult<bytes::Bytes>;

    // ========== Fast Path: MediaTrack Methods (WebRTC Native) ==========

    /// Register a WebRTC native media track callback
    ///
    /// When media samples arrive on the specified track, the registered callback will be invoked.
    /// Uses WebRTC native RTCTrackRemote, no protobuf serialization overhead.
    ///
    /// # Parameters
    ///
    /// - `track_id`: Media track identifier (must match WebRTC track ID in SDP)
    /// - `callback`: Handler function that receives native media samples
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use actr_framework::MediaSample;
    ///
    /// ctx.register_media_track("video-track-1", |sample, sender| {
    ///     Box::pin(async move {
    ///         // Decode and render video frame (native RTP payload)
    ///         println!("Received {} bytes at timestamp {}",
    ///                  sample.data.len(), sample.timestamp);
    ///         decoder.decode(&sample.data).await?;
    ///         Ok(())
    ///     })
    /// }).await?;
    /// ```
    ///
    /// # Architecture Note
    ///
    /// MediaTrack uses WebRTC native RTP channels (RTCTrackRemote), NOT DataChannel.
    /// This provides:
    /// - Zero protobuf serialization overhead
    /// - Native RTP header information (timestamp, SSRC, etc.)
    /// - Optimal latency (~1-2ms lower than DataChannel)
    async fn register_media_track<F>(&self, track_id: String, callback: F) -> ActorResult<()>
    where
        F: Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync + 'static;

    /// Unregister a media track callback
    ///
    /// # Parameters
    ///
    /// - `track_id`: Media track identifier to unregister
    async fn unregister_media_track(&self, track_id: &str) -> ActorResult<()>;

    /// Send media samples via WebRTC native track
    ///
    /// Sends raw media samples through WebRTC RTCRtpSender (native RTP).
    /// This is much more efficient than sending through DataChannel.
    ///
    /// # Parameters
    ///
    /// - `target`: Target destination
    /// - `track_id`: Track identifier
    /// - `sample`: Media sample to send
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use actr_framework::{MediaSample, MediaType};
    ///
    /// let sample = MediaSample {
    ///     data: encoded_frame.into(),
    ///     timestamp: rtp_timestamp,
    ///     codec: "H264".to_string(),
    ///     media_type: MediaType::Video,
    /// };
    ///
    /// ctx.send_media_sample(&target, "video-track-1", sample).await?;
    /// ```
    async fn send_media_sample(
        &self,
        target: &crate::Dest,
        track_id: &str,
        sample: MediaSample,
    ) -> ActorResult<()>;

    /// Add a media track to the WebRTC connection with the target
    ///
    /// Creates a new RTP track on the PeerConnection and triggers SDP renegotiation.
    /// Must be called before `send_media_sample()` for the given track.
    ///
    /// # Parameters
    ///
    /// - `target`: Target destination
    /// - `track_id`: Media track identifier
    /// - `codec`: Codec name (e.g., "VP8", "H264", "OPUS")
    /// - `media_type`: Media type ("video" or "audio")
    async fn add_media_track(
        &self,
        target: &crate::Dest,
        track_id: &str,
        codec: &str,
        media_type: &str,
    ) -> ActorResult<()>;

    /// Remove a media track from the WebRTC connection with the target.
    ///
    /// If the track exists, this removes the RTP sender from the PeerConnection
    /// and triggers SDP renegotiation so repeated start/stop cycles do not keep
    /// stale tracks alive on the connection.
    async fn remove_media_track(&self, target: &crate::Dest, track_id: &str) -> ActorResult<()>;
}

/// Media sample data from WebRTC native track
///
/// Lightweight wrapper around WebRTC native RTP sample.
#[derive(Clone)]
pub struct MediaSample {
    /// Raw sample data (encoded audio/video frame)
    pub data: bytes::Bytes,

    /// Sample timestamp (from RTP timestamp)
    pub timestamp: u32,

    /// Codec-specific information
    pub codec: String,

    /// Media type (audio or video)
    pub media_type: MediaType,
}

/// Media type enum
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Video,
}
