//! ActrSystem - Generic-free infrastructure

use actr_config::Config;
use actr_framework::Workload;
use actr_protocol::ActorResult;
use std::sync::Arc;

use super::ActrNode;
use crate::context_factory::ContextFactory;

// Use types from sub-crates
use crate::wire::webrtc::{
    ReconnectConfig, SignalingClient, SignalingConfig, WebSocketSignalingClient,
};
use actr_runtime_mailbox::{DeadLetterQueue, Mailbox};

/// Network event channels tuple type: (event receiver, result sender, optional debounce config)
type NetworkEventChannels = std::sync::Mutex<
    Option<(
        tokio::sync::mpsc::Receiver<crate::lifecycle::network_event::NetworkEvent>,
        tokio::sync::mpsc::Sender<crate::lifecycle::network_event::NetworkEventResult>,
        Option<crate::lifecycle::network_event::DebounceConfig>,
    )>,
>;

/// ActrSystem - Runtime infrastructure (generic-free)
///
/// # Design Philosophy
/// - Phase 1: Create pure runtime framework
/// - Knows nothing about business logic types
/// - Transforms into ActrNode<W> via attach()
pub struct ActrSystem {
    /// Runtime configuration
    config: Config,

    /// SQLite persistent mailbox
    mailbox: Arc<dyn Mailbox>,

    /// Dead Letter Queue for poison messages
    dlq: Arc<dyn DeadLetterQueue>,

    /// Context factory (with inproc_gate ready, outproc_gate deferred)
    context_factory: ContextFactory,

    /// Signaling client
    signaling_client: Arc<dyn SignalingClient>,

    /// Network event channels (延迟创建，在 create_network_event_handle() 时创建)
    /// attach() 时会 take 这些 channels 传递给 ActrNode
    network_event_channels: NetworkEventChannels,
}

impl ActrSystem {
    /// Create new ActrSystem
    ///
    /// # Errors
    /// - Mailbox initialization failed
    /// - Transport initialization failed
    pub async fn new(config: Config) -> ActorResult<Self> {
        tracing::info!("🚀 Initializing ActrSystem");

        // Initialize Mailbox (using SqliteMailbox implementation)
        let mailbox_path = config
            .mailbox_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ":memory:".to_string());

        tracing::info!("📂 Mailbox database path: {}", mailbox_path);

        let mailbox: Arc<dyn Mailbox> = Arc::new(
            actr_runtime_mailbox::SqliteMailbox::new(&mailbox_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("Mailbox init failed: {e}"))
                })?,
        );

        // Initialize Dead Letter Queue
        // Use same path as mailbox, DLQ will create separate table in same database
        let dlq_path = if mailbox_path == ":memory:" {
            ":memory:".to_string() // Separate in-memory DB for DLQ
        } else {
            format!("{mailbox_path}.dlq") // Separate file for DLQ
        };

        let dlq: Arc<dyn DeadLetterQueue> = Arc::new(
            actr_runtime_mailbox::SqliteDeadLetterQueue::new_standalone(&dlq_path)
                .await
                .map_err(|e| {
                    actr_protocol::ActrError::Unavailable(format!("DLQ init failed: {e}"))
                })?,
        );
        tracing::info!("✅ Dead Letter Queue initialized");

        // Initialize signaling client (using WebSocketSignalingClient implementation)
        let webrtc_role = if config.webrtc.advanced.prefer_answerer() {
            Some("answer".to_string())
        } else {
            None
        };

        let signaling_config = SignalingConfig {
            server_url: config.signaling_url.clone(),
            connection_timeout: 30,
            heartbeat_interval: 30,
            reconnect_config: ReconnectConfig::default(),
            auth_config: None,
            webrtc_role,
        };

        let client = Arc::new(WebSocketSignalingClient::new(signaling_config));
        client.start_reconnect_manager(); // Start if reconnect_config.enabled = true
        let signaling_client: Arc<dyn SignalingClient> = client;

        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        // Initialize inproc infrastructure (Shell/Local communication - immediately available)
        // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
        use crate::outbound::{InprocOutGate, OutGate};
        use crate::transport::InprocTransportManager;

        // Create TWO separate InprocTransportManager instances for bidirectional communication
        // This ensures Shell's pending_requests and Workload's pending_requests are separate

        // Direction 1: Shell → Workload (REQUEST)
        let shell_to_workload = Arc::new(InprocTransportManager::new());
        tracing::debug!("✨ Created shell_to_workload InprocTransportManager");

        // Direction 2: Workload → Shell (RESPONSE)
        let workload_to_shell = Arc::new(InprocTransportManager::new());
        tracing::debug!("✨ Created workload_to_shell InprocTransportManager");

        // Shell uses shell_to_workload for sending
        let inproc_gate =
            OutGate::InprocOut(Arc::new(InprocOutGate::new(shell_to_workload.clone())));

        // Create DataStreamRegistry for DataStream callbacks
        let data_stream_registry = Arc::new(crate::inbound::DataStreamRegistry::new());
        tracing::debug!("✨ Created DataStreamRegistry");

        // Create MediaFrameRegistry for MediaTrack callbacks
        let media_frame_registry = Arc::new(crate::inbound::MediaFrameRegistry::new());
        tracing::debug!("✨ Created MediaFrameRegistry");

        // ContextFactory holds both managers and registries
        let context_factory = ContextFactory::new(
            inproc_gate,
            shell_to_workload.clone(),
            workload_to_shell.clone(),
            data_stream_registry,
            media_frame_registry,
            signaling_client.clone(),
        );

        tracing::info!("✅ Inproc infrastructure initialized (bidirectional Shell ↔ Workload)");

        tracing::info!("✅ ActrSystem initialized");

        Ok(Self {
            config,
            mailbox,
            dlq,
            context_factory,
            signaling_client,
            network_event_channels: std::sync::Mutex::new(None),
        })
    }

    /// 创建网络事件处理基础设施（按需调用）
    ///
    /// 创建 NetworkEventHandle 和内部 channels。
    /// channels 会存储在结构体中，供 attach() 使用。
    ///
    /// # 参数
    /// - `debounce_ms`: 防抖窗口时间（毫秒）。如果为 0，则使用默认值。
    ///
    /// # 注意
    /// - 只能调用一次
    /// - 如果不调用此方法，网络事件功能将不可用
    ///
    /// # Panics
    /// 如果已经调用过此方法，会 panic
    pub fn create_network_event_handle(
        &self,
        debounce_ms: u64,
    ) -> crate::lifecycle::NetworkEventHandle {
        // 创建双向 channels
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);
        let (result_tx, result_rx) = tokio::sync::mpsc::channel(100);

        // 存储 channels（供 attach() 使用）
        let mut channels = self
            .network_event_channels
            .lock()
            .expect("Failed to lock network_event_channels");

        if channels.is_some() {
            panic!("create_network_event_handle() can only be called once");
        }

        // 构造防抖配置
        let debounce_config = if debounce_ms > 0 {
            Some(crate::lifecycle::network_event::DebounceConfig {
                window: std::time::Duration::from_millis(debounce_ms),
            })
        } else {
            None
        };

        *channels = Some((event_rx, result_tx, debounce_config));

        // 创建并返回 NetworkEventHandle
        crate::lifecycle::NetworkEventHandle::new(event_tx, result_rx)
    }

    /// Attach Workload, transform into ActrNode<W>
    ///
    /// # Type Inference
    /// - Infer W::Dispatcher from W
    /// - Compiler monomorphizes ActrNode<W>
    /// - Completely zero-dyn, full inline chain
    ///
    /// # Consumes self
    /// - Move ensures can only be called once
    /// - Embodies one-actor-per-instance principle
    pub fn attach<W: Workload>(self, workload: W) -> ActrNode<W> {
        tracing::info!("📦 Attaching workload");

        // Try to load Actr.lock.toml from config directory (optional)
        let actr_lock_path = self.config.config_dir.join("Actr.lock.toml");
        let actr_lock = match actr_config::lock::LockFile::from_file(&actr_lock_path) {
            Ok(lock) => {
                tracing::info!(
                    "📋 Loaded Actr.lock.toml with {} dependencies",
                    lock.dependencies.len()
                );
                Some(lock)
            }
            Err(e) => {
                // If lock file is missing or invalid, continue without dependency fingerprints
                tracing::warn!(
                    "⚠️ Actr.lock.toml not loaded (path: {:?}, ERR: {}). Continuing without dependency fingerprints.",
                    actr_lock_path,
                    e
                );
                None
            }
        };
        // 从 network_event_channels 中 take channels（如果存在）
        let (network_event_rx, network_event_result_tx, network_event_debounce_config) = self
            .network_event_channels
            .lock()
            .expect("Failed to lock network_event_channels")
            .take()
            .map(|(rx, tx, config)| (Some(rx), Some(tx), config))
            .unwrap_or((None, None, None));

        ActrNode {
            config: self.config,
            workload: Arc::new(workload),
            mailbox: self.mailbox,
            dlq: self.dlq,
            context_factory: Some(self.context_factory), // Initialized with inproc_gate ready
            signaling_client: self.signaling_client,
            actor_id: None,              // Obtained after startup
            credential_state: None,      // Obtained after startup（含 TurnCredential）
            webrtc_coordinator: None,    // Pass shared coordinator
            webrtc_gate: None,           // Created after startup
            websocket_gate: None,        // Created after startup (if websocket_listen_port is set)
            inproc_mgr: None,            // Set after startup
            workload_to_shell_mgr: None, // Set after startup
            shutdown_token: tokio_util::sync::CancellationToken::new(),
            actr_lock,
            network_event_rx,
            network_event_result_tx,
            network_event_debounce_config,
            dedup_state: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::lifecycle::dedup::DedupState::new(),
            )),
            discovered_ws_addresses: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
        }
    }
}
