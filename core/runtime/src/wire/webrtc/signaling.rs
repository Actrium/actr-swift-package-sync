//! signaling clientImplementation
//!
//! Based on protobuf Definition'ssignalingprotocol, using SignalingEnvelope conclude construct

#[cfg(feature = "opentelemetry")]
use super::trace;
use crate::lifecycle::CredentialState;
use crate::transport::error::{NetworkError, NetworkResult};
#[cfg(feature = "opentelemetry")]
use crate::wire::webrtc::trace::extract_trace_context;
#[cfg(feature = "opentelemetry")]
use actr_protocol::ActrIdExt;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrToSignaling, CredentialUpdateRequest, GetSigningKeyRequest,
    PeerToSignaling, Ping, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, ServiceAvailabilityState, SignalingEnvelope, UnregisterRequest,
    UnregisterResponse, actr_to_signaling, peer_to_signaling, signaling_envelope,
    signaling_to_actr,
};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};
#[cfg(feature = "opentelemetry")]
use tracing_opentelemetry::OpenTelemetrySpanExt;
use url::Url;

/// WebSocket sink type alias for the split write half of a signaling connection
type WsSink = Arc<
    tokio::sync::Mutex<
        Option<
            futures_util::stream::SplitSink<
                WebSocketStream<MaybeTlsStream<TcpStream>>,
                tokio_tungstenite::tungstenite::Message,
            >,
        >,
    >,
>;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default timeout in seconds for waiting for signaling response
const RESPONSE_TIMEOUT_SECS: u64 = 15;
// WebSocket-level keepalive to detect silent half-open connections
const PING_INTERVAL_SECS: u64 = 5;
const PONG_TIMEOUT_SECS: u64 = 10;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// configurationType
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// signalingconfiguration
#[derive(Debug, Clone)]
pub struct SignalingConfig {
    /// signaling server URL
    pub server_url: Url,

    /// Connecttimeout temporal duration （seconds）
    pub connection_timeout: u64,

    /// center skipinterval（seconds）
    pub heartbeat_interval: u64,

    /// reconnection configuration
    pub reconnect_config: ReconnectConfig,

    /// acknowledge verify configuration
    pub auth_config: Option<AuthConfig>,

    /// WebRTC role preference: "answer" if this node has advanced config
    pub webrtc_role: Option<String>,
}

/// reconnection configuration
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// whether start usage automatic reconnection
    pub enabled: bool,

    /// maximum reconnection attempts
    pub max_attempts: u32,

    /// initial reconnection delay（seconds）
    pub initial_delay: u64,

    /// maximum reconnection delay（seconds）
    pub max_delay: u64,

    /// Backoff multiplier factor
    pub backoff_multiplier: f64,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            initial_delay: 1,
            max_delay: 60,
            backoff_multiplier: 2.0,
        }
    }
}

/// acknowledge verify configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// acknowledge verify Type
    pub auth_type: AuthType,

    /// acknowledge verify credential data
    pub credentials: HashMap<String, String>,
}

/// acknowledge verify Type
#[derive(Debug, Clone)]
pub enum AuthType {
    /// no acknowledge verify
    None,
    /// Bearer Token
    BearerToken,
    /// API Key
    ApiKey,
    /// JWT
    Jwt,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Client interface and implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// signaling client connect port
///
/// # interior mutability
/// allMethodusing `&self` and non `&mut self`, with for conveniencein Arc in shared.
/// Implementation class needs interior mutability （ like Mutex）to manage WebSocket connection status.
#[async_trait]
pub trait SignalingClient: Send + Sync {
    /// Connecttosignaling server
    async fn connect(&self) -> NetworkResult<()>;

    /// DisconnectConnect
    async fn disconnect(&self) -> NetworkResult<()>;

    /// SendRegisterrequest（Register front stream process, using PeerToSignaling）
    async fn send_register_request(
        &self,
        request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse>;

    /// Send UnregisterRequest to signaling server (Actr → Signaling flow)
    ///
    /// This is used when an Actor is shutting down gracefully and wants to
    /// proactively notify the signaling server that it is no longer available.
    async fn send_unregister_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse>;

    /// Send center skip（Registerafter stream process, using ActrToSignaling）
    /// Returns Pong response if received, error if timeout or no response
    async fn send_heartbeat(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        availability: ServiceAvailabilityState,
        power_reserve: f32,
        mailbox_backlog: f32,
    ) -> NetworkResult<Pong>;

    /// Send RouteCandidatesRequest (requires authenticated Actor session)
    async fn send_route_candidates_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse>;

    /// 向 signaling 查询 AIS 的 Ed25519 signing 公钥
    ///
    /// 返回 `(key_id, pubkey_bytes)`，其中 pubkey_bytes 为 32 字节原始公钥。
    /// 通常由 AisKeyCache 在缓存 miss 时调用，不应在热路径中直接使用。
    async fn get_signing_key(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)>;

    /// Send CredentialUpdateRequest to refresh the Actor's credential
    ///
    /// This is used to refresh the credential before it expires. The server responds
    /// with a RegisterResponse containing the new credential and expiration time.
    async fn send_credential_update_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
    ) -> NetworkResult<RegisterResponse>;

    /// Sendsignalingsignal seal （ pass usage Method）
    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()>;

    /// Receivesignalingsignal seal
    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>>;

    /// Check connection status
    fn is_connected(&self) -> bool;

    /// GetConnect statistics info
    fn get_stats(&self) -> SignalingStats;
    /// Subscribe to signaling events (state transitions).
    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent>;

    /// Set actor ID and credential state for reconnect URL parameters.
    async fn set_actor_id(&self, actor_id: ActrId);
    async fn set_credential_state(&self, credential_state: CredentialState);

    /// Clear stored actor ID and credential state.
    ///
    /// After calling this, `connect()` will produce a clean WebSocket URL
    /// without identity query parameters, so the signaling server treats
    /// the connection as brand-new rather than a reconnect of the old actor.
    /// This is required before re-registration when the credential has expired.
    async fn clear_identity(&self);

    /// Set a lifecycle hook callback that will be invoked (and awaited)
    /// whenever signaling state changes (connect/disconnect).
    /// Default implementation is a no-op for clients that don't support hooks.
    fn set_hook_callback(&self, _cb: HookCallback) {}
}

/// High-level signaling connection state (kept for quick boolean checks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connected,
}

/// Signaling state transition events.
///
/// Unlike `ConnectionState` (which is a snapshot), these represent discrete
/// transitions and are delivered via `broadcast` so every subscriber sees
/// every event, even if the same state occurs twice in a row.
#[derive(Debug, Clone)]
pub enum SignalingEvent {
    /// About to start a connection attempt (includes retry count).
    ConnectStart { attempt: u32 },
    /// Connection successfully established.
    Connected,
    /// Connection lost.
    Disconnected { reason: DisconnectReason },
}

/// Reason why the signaling connection was lost.
#[derive(Debug, Clone)]
pub enum DisconnectReason {
    /// WebSocket stream ended (receiver task exited normally).
    StreamEnded,
    /// No Pong received within the timeout window.
    PongTimeout,
    /// Failed to send a WebSocket Ping frame.
    PingSendFailed,
    /// Credential expired (heartbeat 401).
    CredentialExpired,
    /// Explicit disconnect() call or external trigger.
    Manual,
    /// Connection attempt failed with an error.
    ConnectionFailed(String),
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Hook callback for synchronous lifecycle notification
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Events that trigger workload lifecycle hooks.
///
/// Used by `HookCallback` to invoke workload hooks synchronously (awaited)
/// at the point where the state change occurs.
#[derive(Clone, Debug)]
pub enum HookEvent {
    // ── Signaling ──
    SignalingConnectStart { attempt: u32 },
    SignalingConnected,
    SignalingDisconnected,
    // ── WebRTC ──
    WebRtcConnectStart { peer_id: ActrId },
    WebRtcConnected { peer_id: ActrId, relayed: bool },
    WebRtcDisconnected { peer_id: ActrId },
}

/// Callback closure that is awaited when a hook event occurs.
///
/// Set once via `set_hook_callback()`. All state-change paths invoke this
/// closure and `.await` its result before proceeding.
pub type HookCallback =
    Arc<dyn Fn(HookEvent) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// WebSocket signaling clientImplementation
pub struct WebSocketSignalingClient {
    config: SignalingConfig,
    actor_id: tokio::sync::Mutex<Option<ActrId>>,
    credential_state: tokio::sync::Mutex<Option<CredentialState>>,
    /// WebSocket write end （using Mutex Implementation interior mutability ）
    ws_sink: WsSink,
    /// WebSocket read end （using Mutex Implementation interior mutability ）
    ws_stream: tokio::sync::Mutex<
        Option<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    >,
    /// connection status
    connected: Arc<AtomicBool>,
    /// Connection in progress flag (prevents concurrent connect attempts)
    connecting: Arc<AtomicBool>,
    /// statistics info
    stats: Arc<AtomicSignalingStats>,
    /// Envelope count number device
    envelope_counter: tokio::sync::Mutex<u64>,
    /// Pending reply waiters (reply_for -> oneshot)
    pending_replies: Arc<tokio::sync::Mutex<HashMap<String, oneshot::Sender<SignalingEnvelope>>>>,
    /// Inbound envelope channel for unmatched messages (ActrRelay / push)
    inbound_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<SignalingEnvelope>>>,
    inbound_tx: tokio::sync::Mutex<mpsc::UnboundedSender<SignalingEnvelope>>,
    /// Background receive task handle to allow graceful shutdown
    receiver_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Background ping task to detect half-open connections
    ping_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Connection state broadcast channel (event-driven)
    event_tx: broadcast::Sender<SignalingEvent>,
    /// Last time we saw inbound traffic (pong/any message), unix epoch seconds
    last_pong: Arc<AtomicU64>,
    /// Flag to track if reconnect manager has been started
    reconnector_started: Arc<AtomicBool>,
    /// Notify channel to wake up the reconnect manager
    reconnect_notify: Arc<tokio::sync::Notify>,
    /// Hook callback for synchronous lifecycle notification (set once, lock-free read)
    hook_callback: OnceLock<HookCallback>,
}

impl WebSocketSignalingClient {
    /// Create new WebSocket signaling client
    pub fn new(config: SignalingConfig) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = broadcast::channel(64);
        Self {
            config,
            actor_id: tokio::sync::Mutex::new(None),
            credential_state: tokio::sync::Mutex::new(None),
            ws_sink: Arc::new(tokio::sync::Mutex::new(None)),
            ws_stream: tokio::sync::Mutex::new(None),
            connected: Arc::new(AtomicBool::new(false)),
            connecting: Arc::new(AtomicBool::new(false)),
            stats: Arc::new(AtomicSignalingStats::default()),
            envelope_counter: tokio::sync::Mutex::new(0),
            pending_replies: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            inbound_rx: Arc::new(tokio::sync::Mutex::new(inbound_rx)),
            inbound_tx: tokio::sync::Mutex::new(inbound_tx),
            receiver_task: Arc::new(tokio::sync::Mutex::new(None)),
            ping_task: tokio::sync::Mutex::new(None),
            event_tx,
            last_pong: Arc::new(AtomicU64::new(0)),
            reconnector_started: Arc::new(AtomicBool::new(false)),
            reconnect_notify: Arc::new(tokio::sync::Notify::new()),
            hook_callback: OnceLock::new(),
        }
    }

    /// Start the reconnect manager if enabled in config and not already started.
    ///
    /// The manager waits on a `Notify` and runs an exponential-backoff retry loop
    /// each time it is woken up.
    /// Set the hook callback (once). Subsequent calls are silently ignored.
    pub fn set_hook_callback(&self, cb: HookCallback) {
        let _ = self.hook_callback.set(cb);
    }

    /// Invoke the hook callback and await its completion.
    /// No-op if no callback has been set yet.
    async fn invoke_hook(&self, event: HookEvent) {
        if let Some(cb) = self.hook_callback.get() {
            cb(event).await;
        }
    }

    pub fn start_reconnect_manager(self: &Arc<Self>) {
        if !self.config.reconnect_config.enabled {
            return;
        }
        if self
            .reconnector_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return; // already started
        }

        tracing::info!("🔄 Starting reconnect manager for signaling client");

        let client = self.clone();
        let notify = self.reconnect_notify.clone();

        tokio::spawn(async move {
            loop {
                // Wait until someone requests a reconnect
                notify.notified().await;

                if !client.config.reconnect_config.enabled {
                    break;
                }

                // Run reconnect cycle with exponential backoff
                client.run_reconnect_cycle().await;
            }
        });
    }

    /// Execute one full reconnect cycle with exponential backoff + jitter.
    async fn run_reconnect_cycle(self: &Arc<Self>) {
        use actr_framework::ExponentialBackoff;

        let cfg = &self.config.reconnect_config;

        // Cleanup old WebSocket resources first
        tracing::debug!("🧹 Cleaning up old WebSocket resources before reconnect");
        if let Err(e) = self.disconnect().await {
            tracing::warn!("⚠️ Disconnect cleanup failed (non-fatal): {e}");
        }

        let backoff = ExponentialBackoff::builder()
            .initial_delay(std::time::Duration::from_secs(cfg.initial_delay.max(1)))
            .max_delay(std::time::Duration::from_secs(cfg.max_delay.max(1)))
            .max_retries(cfg.max_attempts)
            .with_jitter()
            .build();

        let mut attempt: u32 = 0;

        for delay in backoff {
            if self.connected.load(Ordering::Acquire) {
                tracing::debug!("Already connected, aborting reconnect cycle");
                return;
            }

            attempt += 1;
            let _ = self.event_tx.send(SignalingEvent::ConnectStart { attempt });

            match self.establish_connection_once().await {
                Ok(()) => {
                    tracing::info!("✅ Signaling reconnect succeeded on attempt {attempt}");
                    self.start_receiver().await;
                    self.start_ping_task().await;
                    return;
                }
                Err(e) => {
                    tracing::warn!(
                        "❌ Reconnect attempt {attempt} failed: {e}, retrying in {delay:?}"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }

        // All retries exhausted — enter cooldown, then allow future wakeups
        tracing::error!("Reconnect failed after {attempt} attempts, entering cooldown");
        let cooldown = std::time::Duration::from_secs(cfg.max_delay.max(1) * 2);
        tokio::time::sleep(cooldown).await;
        // After cooldown, the loop returns to notify.notified() and can be woken again
    }

    /// simple for convenience construct create Function
    pub async fn connect_to(url: &str) -> NetworkResult<Arc<Self>> {
        let config = SignalingConfig {
            server_url: url.parse()?,
            connection_timeout: 5,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
            webrtc_role: None,
        };

        let client = Arc::new(Self::new(config));
        client.start_reconnect_manager();
        client.connect().await?;
        Ok(client)
    }

    /// alive integrate down a envelope ID
    async fn next_envelope_id(&self) -> String {
        let mut counter = self.envelope_counter.lock().await;
        *counter += 1;
        format!("env-{}", *counter)
    }

    /// Create SignalingEnvelope
    async fn create_envelope(&self, flow: signaling_envelope::Flow) -> SignalingEnvelope {
        SignalingEnvelope {
            envelope_version: 1,
            envelope_id: self.next_envelope_id().await,
            reply_for: None,
            timestamp: prost_types::Timestamp {
                seconds: chrono::Utc::now().timestamp(),
                nanos: 0,
            },
            traceparent: None,
            tracestate: None,
            flow: Some(flow),
        }
    }

    /// Reset inbound channel for a fresh session (useful after disconnects).
    async fn reset_inbound_channel(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        *self.inbound_tx.lock().await = tx;
        *self.inbound_rx.lock().await = rx;
    }

    /// Build signaling URL with actor identity and Ed25519 credential params for authentication.
    ///
    /// Passes `actor_id`, `key_id`, `claims` (base64), `signature` (base64) as URL query params
    /// so the signaling server can validate the credential before upgrading the WebSocket.
    async fn build_url_with_identity(&self) -> Url {
        let mut url = self.config.server_url.clone();
        let actor_id_opt = self.actor_id.lock().await.clone();
        if let Some(actor_id) = actor_id_opt {
            let actor_str = actr_protocol::ActrIdExt::to_string_repr(&actor_id);
            url.query_pairs_mut().append_pair("actor_id", &actor_str);
        }

        // Pass Ed25519 credential in URL for initial WS auth
        let cred_state_opt = self.credential_state.lock().await.clone();
        if let Some(cred_state) = cred_state_opt {
            let cred = cred_state.credential().await;
            let claims_b64 = base64::engine::general_purpose::STANDARD.encode(&cred.claims);
            let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&cred.signature);
            url.query_pairs_mut()
                .append_pair("key_id", &cred.key_id.to_string())
                .append_pair("claims", &claims_b64)
                .append_pair("signature", &sig_b64);
        }

        // Add WebRTC role preference if configured
        if let Some(role) = &self.config.webrtc_role {
            url.query_pairs_mut().append_pair("webrtc_role", role);
        }

        url
    }

    /// Establish a single signaling WebSocket connection attempt, honoring connection_timeout.
    ///
    /// This does not perform any retry logic; callers that want retries should wrap this.
    async fn establish_connection_once(&self) -> NetworkResult<()> {
        let url = self.build_url_with_identity().await;
        let timeout_secs = self.config.connection_timeout;
        tracing::debug!("Establishing connection to URL: {}", url.as_str());
        // 断网后，写入到缓冲区的数据，网络恢复后会继续发送
        let config = WebSocketConfig::default().write_buffer_size(0);
        // Connect with an optional timeout. A timeout of 0 means "no timeout".
        let connect_result = if timeout_secs == 0 {
            connect_async_with_config(url.as_str(), Some(config), false).await
        } else {
            let timeout_duration = std::time::Duration::from_secs(timeout_secs);
            tokio::time::timeout(
                timeout_duration,
                connect_async_with_config(url.as_str(), Some(config), false),
            )
            .await
            .map_err(|_| {
                NetworkError::ConnectionError(format!(
                    "Signaling connect timeout after {}s",
                    timeout_secs
                ))
            })?
        }?;

        let (ws_stream, _) = connect_result;

        // Split read/write halves and initialize client state
        let (sink, stream) = ws_stream.split();

        *self.ws_sink.lock().await = Some(sink);
        *self.ws_stream.lock().await = Some(stream);
        self.connected.store(true, Ordering::Release);
        self.last_pong.store(current_unix_secs(), Ordering::Release);
        // Invoke hook synchronously, then broadcast for other subscribers
        self.invoke_hook(HookEvent::SignalingConnected).await;
        let _ = self.event_tx.send(SignalingEvent::Connected);

        self.stats.connections.fetch_add(1, Ordering::Relaxed);

        Ok(())
    }

    /// Connect to signaling server with retry and exponential backoff based on reconnect_config.
    async fn connect_with_retries(&self) -> NetworkResult<()> {
        use actr_framework::ExponentialBackoff;

        let cfg = &self.config.reconnect_config;

        // If reconnect is disabled, just attempt once.
        if !cfg.enabled {
            return self.establish_connection_once().await;
        }

        let backoff = ExponentialBackoff::builder()
            .initial_delay(std::time::Duration::from_secs(cfg.initial_delay.max(1)))
            .max_delay(std::time::Duration::from_secs(cfg.max_delay.max(1)))
            .max_retries(cfg.max_attempts)
            .with_jitter()
            .build();

        let mut last_err = None;

        // 首次立即尝试（delay = 0），后续由 backoff 产生退避延迟
        for (attempt, delay) in std::iter::once(std::time::Duration::ZERO)
            .chain(backoff)
            .enumerate()
        {
            let attempt = attempt as u32 + 1;
            self.invoke_hook(HookEvent::SignalingConnectStart { attempt })
                .await;
            if delay > std::time::Duration::ZERO {
                tracing::info!("Retry signaling connect after {delay:?} (attempt {attempt})");
                tokio::time::sleep(delay).await;
            }

            match self.establish_connection_once().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!("Signaling connect attempt {attempt} failed: {e:?}");
                    last_err = Some(e);
                }
            }
        }

        let total = cfg.max_attempts + 1; // backoff max_retries + 首次
        tracing::error!("Signaling connect failed after {total} attempts, giving up");
        Err(last_err.unwrap_or_else(|| {
            NetworkError::ConnectionError("All connection attempts failed".to_string())
        }))
    }

    /// Send envelope and wait for response with timeout and error handling.
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, fields(envelope_id = %envelope.envelope_id))
    )]
    async fn send_envelope_and_wait_response(
        &self,
        envelope: SignalingEnvelope,
    ) -> NetworkResult<SignalingEnvelope> {
        let reply_for = envelope.envelope_id.clone();

        // Register waiter before sending
        let (tx, rx) = oneshot::channel();
        self.pending_replies
            .lock()
            .await
            .insert(reply_for.clone(), tx);

        if let Err(e) = self.send_envelope(envelope).await {
            // Cleanup waiter on immediate send failure to avoid leaks.
            self.pending_replies.lock().await.remove(&reply_for);
            return Err(e);
        }

        let result =
            tokio::time::timeout(std::time::Duration::from_secs(RESPONSE_TIMEOUT_SECS), rx).await;
        // Clean up waiter on timeout
        if result.is_err() {
            self.pending_replies.lock().await.remove(&reply_for);
        }

        let response_envelope = result
            .map_err(|_| {
                NetworkError::ConnectionError(
                    "Timed out waiting for signaling response".to_string(),
                )
            })?
            .map_err(|_| {
                NetworkError::ConnectionError(
                    "Receiver dropped while waiting for signaling response".to_string(),
                )
            })?;

        Ok(response_envelope)
    }

    /// Spawn background receiver to demux envelopes by reply_for.
    async fn start_receiver(&self) {
        let mut stream_guard = self.ws_stream.lock().await;
        if stream_guard.is_none() {
            return;
        }

        let mut stream = stream_guard.take().expect("stream exists");
        let pending = self.pending_replies.clone();
        let inbound_tx = { self.inbound_tx.lock().await.clone() };
        let stats = self.stats.clone();
        let connected = self.connected.clone();
        let event_tx = self.event_tx.clone();
        let last_pong = self.last_pong.clone();
        let reconnect_notify = self.reconnect_notify.clone();
        let reconnect_enabled = self.config.reconnect_config.enabled;
        let handle = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                        // Any inbound traffic counts as liveness
                        last_pong.store(current_unix_secs(), Ordering::Release);
                        match SignalingEnvelope::decode(&data[..]) {
                            Ok(envelope) => {
                                #[cfg(feature = "opentelemetry")]
                                let span = {
                                    let span = tracing::info_span!("signaling.receive_envelope", envelope_id = %envelope.envelope_id);
                                    span.set_parent(extract_trace_context(&envelope));
                                    span
                                };

                                stats.messages_received.fetch_add(1, Ordering::Relaxed);
                                tracing::debug!("Received message: {:?}", envelope);
                                if let Some(reply_for) = envelope.reply_for.clone() {
                                    if let Some(sender) = pending.lock().await.remove(&reply_for) {
                                        #[cfg(feature = "opentelemetry")]
                                        let _ = span.enter();
                                        if let Err(e) = sender.send(envelope) {
                                            stats.errors.fetch_add(1, Ordering::Relaxed);
                                            tracing::warn!(
                                                "Failed to send reply envelope to waiter: {e:?}",
                                            );
                                        }
                                        continue;
                                    }
                                }
                                tracing::debug!(
                                    "Unmatched or push message -> forward to inbound channel"
                                );
                                // Unmatched or push message -> forward to inbound channel
                                if let Err(e) = inbound_tx.send(envelope) {
                                    stats.errors.fetch_add(1, Ordering::Relaxed);
                                    tracing::warn!(
                                        "Failed to send envelope to inbound channel: {e:?}"
                                    );
                                }
                            }
                            Err(e) => {
                                stats.errors.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!("Failed to decode SignalingEnvelope: {e}");
                            }
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Pong(_)) => {
                        tracing::debug!("Received pong");
                        last_pong.store(current_unix_secs(), Ordering::Release);
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Ping(_)) => {
                        tracing::debug!("Received ping");
                        last_pong.store(current_unix_secs(), Ordering::Release);
                    }
                    Ok(other) => {
                        tracing::warn!("Received non-binary frame, ignoring: {other:?}");
                    }
                    Err(e) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("Signaling receive error: {e}");
                        break;
                    }
                }
            }

            tracing::warn!("Stream terminated");
            // Stream terminated → mark disconnected and wake reconnect manager
            stats.disconnections.fetch_add(1, Ordering::Relaxed);
            connected.store(false, Ordering::Release);
            let _ = event_tx.send(SignalingEvent::Disconnected {
                reason: DisconnectReason::StreamEnded,
            });
            if reconnect_enabled {
                reconnect_notify.notify_one();
            }
        });

        *self.receiver_task.lock().await = Some(handle);
    }

    /// Spawn background ping task to detect half-open connections where writes do not fail but peer is gone.
    /// fixme: merge to heartbeat task
    async fn start_ping_task(&self) {
        let mut existing = self.ping_task.lock().await;
        if let Some(handle) = existing.as_ref() {
            if handle.is_finished() {
                existing.take();
            } else {
                return;
            }
        }

        let sink = self.ws_sink.clone();
        let connected = self.connected.clone();
        let event_tx = self.event_tx.clone();
        let last_pong = self.last_pong.clone();
        let receiver_task_clone = Arc::clone(&self.receiver_task);
        let reconnect_notify = self.reconnect_notify.clone();
        let reconnect_enabled = self.config.reconnect_config.enabled;

        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(PING_INTERVAL_SECS)).await;

                if !connected.load(Ordering::Acquire) {
                    break;
                }

                // Send ping; mark disconnect on failure.
                let mut sink_guard = sink.lock().await;
                if let Some(sink) = sink_guard.as_mut() {
                    if let Err(e) = sink
                        .send(tokio_tungstenite::tungstenite::Message::Ping(
                            Vec::new().into(),
                        ))
                        .await
                    {
                        tracing::warn!("Signaling ping send failed: {e}");
                        connected.store(false, Ordering::Release);
                        let _ = event_tx.send(SignalingEvent::Disconnected {
                            reason: DisconnectReason::PingSendFailed,
                        });
                        if reconnect_enabled {
                            reconnect_notify.notify_one();
                        }
                        break;
                    }
                } else {
                    tracing::warn!("Signaling not connected");
                    connected.store(false, Ordering::Release);
                    let _ = event_tx.send(SignalingEvent::Disconnected {
                        reason: DisconnectReason::PingSendFailed,
                    });
                    if reconnect_enabled {
                        reconnect_notify.notify_one();
                    }
                    break;
                }
                drop(sink_guard);

                // Check for stale pong
                let now = current_unix_secs();
                let last = last_pong.load(Ordering::Acquire);
                if now.saturating_sub(last) > PONG_TIMEOUT_SECS {
                    tracing::warn!(
                        "Signaling pong timeout (last seen {}s ago), marking disconnected",
                        now.saturating_sub(last)
                    );
                    if let Some(handle) = receiver_task_clone.lock().await.take() {
                        handle.abort();
                    }
                    connected.store(false, Ordering::Release);
                    let _ = event_tx.send(SignalingEvent::Disconnected {
                        reason: DisconnectReason::PongTimeout,
                    });
                    if reconnect_enabled {
                        reconnect_notify.notify_one();
                    }
                    break;
                }
            }
        });

        *existing = Some(handle);
    }

    /// Wait for ongoing connection attempt to complete (used when another task is connecting).
    ///
    /// Uses the broadcast channel to wait for a Connected event without recursion.
    async fn wait_for_connection_result(&self) -> NetworkResult<()> {
        let mut event_rx = self.event_tx.subscribe();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    // Final check before giving up
                    if self.connected.load(Ordering::Acquire) {
                        tracing::debug!("Connection succeeded just before timeout");
                        return Ok(());
                    }
                    return Err(NetworkError::ConnectionError(
                        "Timeout waiting for concurrent connection attempt".to_string(),
                    ));
                }
                result = event_rx.recv() => {
                    match result {
                        Ok(SignalingEvent::Connected) => {
                            tracing::debug!("Connection established by another task");
                            return Ok(());
                        }
                        Ok(_) => continue, // ConnectStart / Disconnected — keep waiting
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Event receiver lagged by {n} events");
                            // Check current state after lag
                            if self.connected.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err(NetworkError::ConnectionError(
                                "Event channel closed while waiting for connection".to_string(),
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl SignalingClient for WebSocketSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        // 🔐 Fast path: Check if already connected
        if self.connected.load(Ordering::Acquire) {
            tracing::debug!("Already connected, skipping connect()");
            return Ok(());
        }

        // 🔐 Try to acquire "connecting" lock using compare-and-swap
        if self
            .connecting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // Another task is connecting, wait for state change using watch channel
            tracing::debug!("Another connection attempt in progress, waiting for state change...");

            return self.wait_for_connection_result().await;
        }

        // 🔐 We now hold the "connecting" lock, proceed with connection
        tracing::debug!("Acquired connection lock, establishing connection...");

        // Perform actual connection
        let result = self.connect_with_retries().await;

        // Clear "connecting" flag regardless of result
        self.connecting.store(false, Ordering::Release);

        // Handle connection result
        match result {
            Ok(()) => {
                self.start_receiver().await;
                self.start_ping_task().await;
                Ok(())
            }
            Err(e) => {
                // Explicitly notify waiting tasks that connection failed
                let _ = self.event_tx.send(SignalingEvent::Disconnected {
                    reason: DisconnectReason::ConnectionFailed(e.to_string()),
                });
                tracing::error!("Connection failed: {e}");
                Err(e)
            }
        }
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        // fetch exit sink and stream
        let mut sink_guard = self.ws_sink.lock().await;
        let mut stream_guard = self.ws_stream.lock().await;

        // Close sink
        if let Some(mut sink) = sink_guard.take() {
            let _ = sink.close().await;
        }

        // clear blank stream
        stream_guard.take();

        // Stop receiver task if running
        if let Some(handle) = self.receiver_task.lock().await.take() {
            handle.abort();
        }
        // Stop ping task if running
        if let Some(handle) = self.ping_task.lock().await.take() {
            handle.abort();
        }

        self.reset_inbound_channel().await;

        self.connected.store(false, Ordering::Release);
        self.stats.disconnections.fetch_add(1, Ordering::Relaxed);

        // Invoke hook synchronously, then broadcast for other subscribers
        self.invoke_hook(HookEvent::SignalingDisconnected).await;
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: DisconnectReason::Manual,
        });

        Ok(())
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn send_register_request(
        &self,
        request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        // Create PeerToSignaling stream process （Register front ）
        let flow = signaling_envelope::Flow::PeerToServer(PeerToSignaling {
            payload: Some(peer_to_signaling::Payload::RegisterRequest(request)),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            if let Some(signaling_to_actr::Payload::RegisterResponse(response)) =
                server_to_actr.payload
            {
                return Ok(response);
            }
        }

        Err(NetworkError::ConnectionError(
            "Invalid registration response".to_string(),
        ))
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, fields(actor_id = %actor_id.to_string_repr()))
    )]
    async fn send_unregister_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        // Build UnregisterRequest payload
        let request = UnregisterRequest {
            actr_id: actor_id.clone(),
            reason,
        };

        // Wrap into ActrToSignaling flow
        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::UnregisterRequest(request)),
        });

        // Send envelope (fire-and-forget)
        let envelope = self.create_envelope(flow).await;
        self.send_envelope(envelope).await?;

        // Do not wait for UnregisterResponse here because the signaling stream
        // is also consumed by WebRtcCoordinator. Waiting could race with that loop
        // and lead to spurious timeouts. Treat Unregister as best-effort.
        // not wait for the response , because the signaling stream have multi customers use it, fixme: should wait for the response
        Ok(UnregisterResponse {
            result: Some(actr_protocol::unregister_response::Result::Success(
                actr_protocol::unregister_response::UnregisterOk {},
            )),
        })
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "debug", skip_all, fields(actor_id = %actor_id.to_string_repr()))
    )]
    async fn send_heartbeat(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        availability: ServiceAvailabilityState,
        power_reserve: f32,
        mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        let ping = Ping {
            availability: availability as i32,
            power_reserve,
            mailbox_backlog,
            sticky_client_ids: vec![], // TODO: Implement sticky session tracking
        };

        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::Ping(ping)),
        });

        let envelope = self.create_envelope(flow).await;
        let reply_for = envelope.envelope_id.clone();

        // Register waiter before sending
        let (tx, rx) = oneshot::channel();
        self.pending_replies
            .lock()
            .await
            .insert(reply_for.clone(), tx);

        if let Err(e) = self.send_envelope(envelope).await {
            // Cleanup waiter on immediate send failure to avoid leaks.
            self.pending_replies.lock().await.remove(&reply_for);
            return Err(e);
        }

        // Wait for response
        let response_envelope = rx.await.map_err(|_| {
            NetworkError::ConnectionError(
                "Receiver dropped while waiting for heartbeat response".to_string(),
            )
        })?;

        // Extract Pong from response, or handle Error response
        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            match server_to_actr.payload {
                Some(signaling_to_actr::Payload::Pong(pong)) => {
                    return Ok(pong);
                }
                Some(signaling_to_actr::Payload::Error(err)) => {
                    // Check if it's a credential expired error (401)
                    if err.code == 401 {
                        return Err(NetworkError::CredentialExpired(err.message));
                    }
                    return Err(NetworkError::AuthenticationError(format!(
                        "{} ({})",
                        err.message, err.code
                    )));
                }
                _ => {}
            }
        }

        Err(NetworkError::ConnectionError(
            "Received response but not a Pong message".to_string(),
        ))
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn send_route_candidates_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::RouteCandidatesRequest(request)),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            match server_to_actr.payload {
                Some(signaling_to_actr::Payload::RouteCandidatesResponse(response)) => {
                    return Ok(response);
                }
                Some(signaling_to_actr::Payload::Error(err)) => {
                    return Err(NetworkError::ServiceDiscoveryError(format!(
                        "{} ({})",
                        err.message, err.code
                    )));
                }
                _ => {}
            }
        }

        Err(NetworkError::ConnectionError(
            "Invalid route candidates response".to_string(),
        ))
    }

    async fn get_signing_key(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
        key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::GetSigningKeyRequest(
                GetSigningKeyRequest { key_id },
            )),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            match server_to_actr.payload {
                Some(signaling_to_actr::Payload::GetSigningKeyResponse(resp)) => {
                    return Ok((resp.key_id, resp.pubkey.to_vec()));
                }
                Some(signaling_to_actr::Payload::Error(err)) => {
                    return Err(NetworkError::ConnectionError(format!(
                        "get_signing_key failed: {} ({})",
                        err.message, err.code
                    )));
                }
                _ => {}
            }
        }

        Err(NetworkError::ConnectionError(
            "get_signing_key: 无效响应".to_string(),
        ))
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "debug", skip_all, fields(actor_id = %actor_id.to_string_repr()))
    )]
    async fn send_credential_update_request(
        &self,
        actor_id: ActrId,
        credential: AIdCredential,
    ) -> NetworkResult<RegisterResponse> {
        let request = CredentialUpdateRequest {
            actr_id: actor_id.clone(),
        };

        let flow = signaling_envelope::Flow::ActrToServer(ActrToSignaling {
            source: actor_id,
            credential,
            payload: Some(actr_to_signaling::Payload::CredentialUpdateRequest(request)),
        });

        let envelope = self.create_envelope(flow).await;
        let response_envelope = self.send_envelope_and_wait_response(envelope).await?;

        if let Some(signaling_envelope::Flow::ServerToActr(server_to_actr)) = response_envelope.flow
        {
            match server_to_actr.payload {
                Some(signaling_to_actr::Payload::RegisterResponse(response)) => {
                    return Ok(response);
                }
                Some(signaling_to_actr::Payload::Error(err)) => {
                    return Err(NetworkError::ConnectionError(format!(
                        "Credential update failed: {} ({})",
                        err.message, err.code
                    )));
                }
                _ => {}
            }
        }

        Err(NetworkError::ConnectionError(
            "Invalid credential update response".to_string(),
        ))
    }

    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(level = "debug", skip_all, fields(envelope_id = %envelope.envelope_id))
    )]
    async fn send_envelope(&self, envelope: SignalingEnvelope) -> NetworkResult<()> {
        #[cfg(feature = "opentelemetry")]
        let envelope = {
            let mut envelope = envelope;
            trace::inject_span_context(&tracing::Span::current(), &mut envelope);
            envelope
        };

        // Check connection state first to avoid sending on stale/closed connections
        // This prevents "Broken pipe" errors when ws_sink exists but connection is dead
        if !self.is_connected() {
            return Err(NetworkError::ConnectionError(
                "Cannot send: WebSocket not connected".to_string(),
            ));
        }

        let mut sink_guard = self.ws_sink.lock().await;

        if let Some(sink) = sink_guard.as_mut() {
            // using protobuf binary serialization
            let mut buf = Vec::new();
            envelope.encode(&mut buf)?;
            let msg = tokio_tungstenite::tungstenite::Message::Binary(buf.into());
            sink.send(msg).await?;

            self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("Stats: {:?}", self.stats.snapshot());
            Ok(())
        } else {
            Err(NetworkError::ConnectionError("Not connected".to_string()))
        }
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        let mut rx = self.inbound_rx.lock().await;
        match rx.recv().await {
            Some(envelope) => Ok(Some(envelope)),
            None => {
                tracing::error!("Inbound channel closed");
                Err(NetworkError::ConnectionError(
                    "Inbound channel closed".to_string(),
                ))
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats.snapshot()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, actor_id: ActrId) {
        *self.actor_id.lock().await = Some(actor_id);
    }

    async fn set_credential_state(&self, credential_state: CredentialState) {
        *self.credential_state.lock().await = Some(credential_state);
    }

    async fn clear_identity(&self) {
        *self.actor_id.lock().await = None;
        *self.credential_state.lock().await = None;
    }

    fn set_hook_callback(&self, cb: HookCallback) {
        let _ = self.hook_callback.set(cb);
    }
}

/// signaling statistics info
#[derive(Debug)]
pub(crate) struct AtomicSignalingStats {
    /// Connect attempts
    pub connections: AtomicU64,

    /// DisconnectConnect attempts
    pub disconnections: AtomicU64,

    /// Send'smessage number
    pub messages_sent: AtomicU64,

    /// Receive'smessage number
    pub messages_received: AtomicU64,

    /// Send's center skip number
    /// TODO: Wire heartbeat counters when heartbeat send/receive paths are instrumented; currently never incremented.
    pub heartbeats_sent: AtomicU64,

    /// Receive's center skip number
    /// TODO: Wire heartbeat counters when heartbeat send/receive paths are instrumented; currently never incremented.
    pub heartbeats_received: AtomicU64,

    /// Error attempts
    pub errors: AtomicU64,
}

impl Default for AtomicSignalingStats {
    fn default() -> Self {
        Self {
            connections: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            heartbeats_sent: AtomicU64::new(0),
            heartbeats_received: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }
}

/// Snapshot of statistics for serialization and reading
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SignalingStats {
    /// Connect attempts
    pub connections: u64,

    /// DisconnectConnect attempts
    pub disconnections: u64,

    /// Send'smessage number
    pub messages_sent: u64,

    /// Receive'smessage number
    pub messages_received: u64,

    /// Send's center skip number
    pub heartbeats_sent: u64,

    /// Receive's center skip number
    pub heartbeats_received: u64,

    /// Error attempts
    pub errors: u64,
}

impl AtomicSignalingStats {
    /// Create a snapshot of current statistics
    pub fn snapshot(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::Relaxed),
            disconnections: self.disconnections.load(Ordering::Relaxed),
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            heartbeats_sent: self.heartbeats_sent.load(Ordering::Relaxed),
            heartbeats_received: self.heartbeats_received.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

fn current_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as UsizeOrdering};

    /// Simple fake SignalingClient implementation for testing the reconnect helper.
    struct FakeSignalingClient {
        event_tx: broadcast::Sender<SignalingEvent>,
        connected: AtomicBool,
        connect_calls: Arc<AtomicUsize>,
        actor_id: tokio::sync::Mutex<Option<ActrId>>,
        credential_state: tokio::sync::Mutex<Option<CredentialState>>,
    }

    #[async_trait]
    impl SignalingClient for FakeSignalingClient {
        async fn connect(&self) -> NetworkResult<()> {
            self.connect_calls.fetch_add(1, UsizeOrdering::SeqCst);
            Ok(())
        }

        async fn disconnect(&self) -> NetworkResult<()> {
            Ok(())
        }

        async fn send_register_request(
            &self,
            _request: RegisterRequest,
        ) -> NetworkResult<RegisterResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_unregister_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _reason: Option<String>,
        ) -> NetworkResult<UnregisterResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_heartbeat(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _availability: ServiceAvailabilityState,
            _power_reserve: f32,
            _mailbox_backlog: f32,
        ) -> NetworkResult<Pong> {
            unimplemented!("not needed in tests");
        }

        async fn send_route_candidates_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _request: RouteCandidatesRequest,
        ) -> NetworkResult<RouteCandidatesResponse> {
            unimplemented!("not needed in tests");
        }

        async fn get_signing_key(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
            _key_id: u32,
        ) -> NetworkResult<(u32, Vec<u8>)> {
            unimplemented!("not needed in tests");
        }

        async fn send_credential_update_request(
            &self,
            _actor_id: ActrId,
            _credential: AIdCredential,
        ) -> NetworkResult<RegisterResponse> {
            unimplemented!("not needed in tests");
        }

        async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
            unimplemented!("not needed in tests");
        }

        async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
            unimplemented!("not needed in tests");
        }

        fn is_connected(&self) -> bool {
            self.connected.load(Ordering::SeqCst)
        }

        fn get_stats(&self) -> SignalingStats {
            SignalingStats::default()
        }

        fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
            self.event_tx.subscribe()
        }

        async fn set_actor_id(&self, actor_id: ActrId) {
            *self.actor_id.lock().await = Some(actor_id);
        }

        async fn set_credential_state(&self, credential_state: CredentialState) {
            *self.credential_state.lock().await = Some(credential_state);
        }

        async fn clear_identity(&self) {
            *self.actor_id.lock().await = None;
            *self.credential_state.lock().await = None;
        }
    }

    fn make_fake_client() -> Arc<FakeSignalingClient> {
        let (event_tx, _erx) = broadcast::channel(64);
        let client = Arc::new(FakeSignalingClient {
            event_tx,
            connected: AtomicBool::new(false),
            connect_calls: Arc::new(AtomicUsize::new(0)),
            actor_id: tokio::sync::Mutex::new(None),
            credential_state: tokio::sync::Mutex::new(None),
        });
        client
    }

    /// Helper: create a minimal SignalingConfig with an unreachable URL.
    fn make_config() -> SignalingConfig {
        SignalingConfig {
            server_url: Url::parse("ws://127.0.0.1:1/signaling/ws").unwrap(),
            connection_timeout: 2,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
            webrtc_role: None,
        }
    }

    /// Helper: create a WebSocketSignalingClient wrapped in Arc
    fn make_ws_client(config: SignalingConfig) -> Arc<WebSocketSignalingClient> {
        Arc::new(WebSocketSignalingClient::new(config))
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 1. 配置默认值
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn test_reconnect_config_defaults() {
        let cfg = ReconnectConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.max_attempts, 10);
        assert_eq!(cfg.initial_delay, 1);
        assert_eq!(cfg.max_delay, 60);
        assert!((cfg.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 2. 初始状态
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[test]
    fn test_websocket_signaling_client_initial_state_disconnected() {
        let client = WebSocketSignalingClient::new(make_config());
        assert!(!client.is_connected(), "新创建的客户端应为 Disconnected");
        assert!(
            !client.connecting.load(Ordering::Acquire),
            "新创建的客户端不应处于 connecting 状态"
        );
        assert!(
            !client.reconnector_started.load(Ordering::Acquire),
            "reconnect manager 不应被自动启动"
        );
    }

    #[test]
    fn test_initial_stats_are_zero() {
        let client = WebSocketSignalingClient::new(make_config());
        let stats = client.get_stats();
        assert_eq!(stats.connections, 0);
        assert_eq!(stats.disconnections, 0);
        assert_eq!(stats.messages_sent, 0);
        assert_eq!(stats.messages_received, 0);
        assert_eq!(stats.errors, 0);
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 3. 重连管理器幂等性
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_reconnect_manager_idempotent() {
        let client = make_ws_client(make_config());

        // 第一次启动应该成功
        client.start_reconnect_manager();
        assert!(
            client.reconnector_started.load(Ordering::Acquire),
            "首次调用后 reconnector_started 应为 true"
        );

        // 第二次调用不应启动新的 manager（CAS 失败）
        client.start_reconnect_manager();
        // 如果出现多个 manager，后续测试会因为重复重连而 flaky，这里主要验证标志位
        assert!(client.reconnector_started.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn test_reconnect_manager_disabled_when_config_disabled() {
        let mut config = make_config();
        config.reconnect_config.enabled = false;
        let client = make_ws_client(config);

        client.start_reconnect_manager();
        assert!(
            !client.reconnector_started.load(Ordering::Acquire),
            "当 reconnect 配置为 disabled 时，不应启动 reconnect manager"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 4. connect() 并发互斥
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_connect_fast_path_when_already_connected() {
        let client = make_ws_client(make_config());
        // 手动设置为已连接
        client.connected.store(true, Ordering::Release);

        // connect() 应该直接返回 Ok 而不建立新连接
        let result = client.connect().await;
        assert!(result.is_ok(), "已连接时 connect() 应返回 Ok");
        // 不应改变 connecting 标志
        assert!(!client.connecting.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn test_connect_sets_connecting_flag() {
        let mut config = make_config();
        config.reconnect_config.enabled = false; // 禁用重试，快速失败
        config.connection_timeout = 1;
        let client = make_ws_client(config);

        // 连接会失败（不可达的地址），但应该正确清理 connecting 标志
        let result = client.connect().await;
        assert!(result.is_err(), "连接不可达地址应该失败");
        assert!(
            !client.connecting.load(Ordering::Acquire),
            "连接失败后 connecting 标志应被清除"
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 5. 事件广播
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_event_subscribe_receives_events() {
        let client = make_ws_client(make_config());
        let mut rx = client.subscribe_events();

        // 手动发送事件
        let _ = client.event_tx.send(SignalingEvent::Connected);

        match tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await {
            Ok(Ok(SignalingEvent::Connected)) => {} // 期望收到 Connected 事件
            other => panic!("期望收到 Connected 事件，但得到 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_disconnect_event_on_connect_failure() {
        let mut config = make_config();
        config.reconnect_config.enabled = false;
        config.connection_timeout = 1;
        let client = make_ws_client(config);
        let mut rx = client.subscribe_events();

        // 连接失败
        let _ = client.connect().await;

        // 应该收到 Disconnected(ConnectionFailed) 事件
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(SignalingEvent::Disconnected {
                reason: DisconnectReason::ConnectionFailed(_),
            })) => {} // 期望
            other => panic!(
                "期望收到 Disconnected(ConnectionFailed) 事件，但得到 {:?}",
                other
            ),
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 6. disconnect() 状态清理
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_disconnect_clears_connected_flag() {
        let client = make_ws_client(make_config());
        // 模拟已连接状态
        client.connected.store(true, Ordering::Release);
        assert!(client.is_connected());

        let result = client.disconnect().await;
        assert!(result.is_ok());
        assert!(!client.is_connected(), "disconnect() 后应该为 Disconnected");
    }

    #[tokio::test]
    async fn test_disconnect_increments_disconnection_stat() {
        let client = make_ws_client(make_config());
        client.connected.store(true, Ordering::Release);

        let stats_before = client.get_stats().disconnections;
        let _ = client.disconnect().await;
        let stats_after = client.get_stats().disconnections;
        assert_eq!(
            stats_after,
            stats_before + 1,
            "disconnect() 应该增加断连计数"
        );
    }

    #[tokio::test]
    async fn test_disconnect_idempotent() {
        let client = make_ws_client(make_config());

        // 即使在未连接状态下调用 disconnect() 也不应 panic
        let r1 = client.disconnect().await;
        let r2 = client.disconnect().await;
        assert!(r1.is_ok());
        assert!(r2.is_ok());
        assert!(!client.is_connected());
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 7. reconnect notify 机制
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_reconnect_notify_wakes_waiter() {
        let notify = Arc::new(tokio::sync::Notify::new());
        let notify_clone = notify.clone();
        let woken = Arc::new(AtomicBool::new(false));
        let woken_clone = woken.clone();

        let handle = tokio::spawn(async move {
            notify_clone.notified().await;
            woken_clone.store(true, Ordering::Release);
        });

        // 确保 waiter 已经注册
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!woken.load(Ordering::Acquire), "尚未通知前不应被唤醒");

        // 触发通知
        notify.notify_one();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(woken.load(Ordering::Acquire), "通知后应被唤醒");

        handle.abort();
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 8. URL 构建测试
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_build_url_without_identity() {
        let config = make_config();
        let expected_base = config.server_url.to_string();
        let client = WebSocketSignalingClient::new(config);

        let url = client.build_url_with_identity().await;
        assert_eq!(
            url.to_string(),
            expected_base,
            "没有设置 actor_id 时 URL 不应包含身份参数"
        );
    }

    #[tokio::test]
    async fn test_build_url_with_webrtc_role() {
        let mut config = make_config();
        config.webrtc_role = Some("answer".to_string());
        let client = WebSocketSignalingClient::new(config);

        let url = client.build_url_with_identity().await;
        assert!(
            url.query().unwrap_or("").contains("webrtc_role=answer"),
            "URL 应包含 webrtc_role 参数，实际 URL: {}",
            url
        );
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 9. inbound channel 重置
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_reset_inbound_channel_creates_fresh_channel() {
        let client = WebSocketSignalingClient::new(make_config());

        // 获取旧 tx，发送一条消息
        {
            let tx = client.inbound_tx.lock().await;
            let _ = tx.send(SignalingEnvelope::default());
        }

        // 重置 channel
        client.reset_inbound_channel().await;

        // 旧消息不应在新 channel 中可见
        let mut rx = client.inbound_rx.lock().await;
        let result = rx.try_recv();
        assert!(result.is_err(), "重置后旧消息不应在新 channel 中可见");
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 10. envelope ID 递增
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_envelope_id_monotonically_increasing() {
        let client = WebSocketSignalingClient::new(make_config());

        let id1 = client.next_envelope_id().await;
        let id2 = client.next_envelope_id().await;
        let id3 = client.next_envelope_id().await;

        assert_eq!(id1, "env-1");
        assert_eq!(id2, "env-2");
        assert_eq!(id3, "env-3");
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 11. send_envelope 未连接时应返回错误
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_send_envelope_fails_when_not_connected() {
        let client = WebSocketSignalingClient::new(make_config());
        let envelope = SignalingEnvelope::default();

        let result = client.send_envelope(envelope).await;
        assert!(result.is_err(), "未连接时 send_envelope 应返回错误");
        match result {
            Err(NetworkError::ConnectionError(msg)) => {
                assert!(
                    msg.contains("not connected") || msg.contains("Not connected"),
                    "错误信息应包含 'not connected'，实际: {}",
                    msg
                );
            }
            other => panic!("期望 ConnectionError，得到 {:?}", other),
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // 12. FakeSignalingClient trait 实现验证
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    #[tokio::test]
    async fn test_fake_client_tracks_connect_calls() {
        let client = make_fake_client();
        assert_eq!(client.connect_calls.load(UsizeOrdering::SeqCst), 0);

        client.connect().await.unwrap();
        client.connect().await.unwrap();
        client.connect().await.unwrap();

        assert_eq!(
            client.connect_calls.load(UsizeOrdering::SeqCst),
            3,
            "FakeSignalingClient 应准确记录 connect 调用次数"
        );
    }
}
