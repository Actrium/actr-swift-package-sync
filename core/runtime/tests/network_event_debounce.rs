use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use actr_protocol::{
    AIdCredential, ActrId, Pong, RegisterRequest, RegisterResponse, RouteCandidatesRequest,
    RouteCandidatesResponse, SignalingEnvelope, UnregisterResponse,
};
use actr_runtime::lifecycle::{
    CredentialState, DebounceConfig, DefaultNetworkEventProcessor, NetworkEventProcessor,
};
use actr_runtime::transport::error::{NetworkError, NetworkResult};
use actr_runtime::wire::webrtc::{SignalingClient, SignalingEvent, SignalingStats};
use tokio::sync::broadcast;

struct FakeSignalingClient {
    connected: AtomicBool,
    connections: AtomicU64,
    disconnections: AtomicU64,
    event_tx: broadcast::Sender<SignalingEvent>,
}

impl FakeSignalingClient {
    fn new() -> Self {
        let (event_tx, _event_rx) = broadcast::channel(64);
        Self {
            connected: AtomicBool::new(false),
            connections: AtomicU64::new(0),
            disconnections: AtomicU64::new(0),
            event_tx,
        }
    }

    fn stats(&self) -> SignalingStats {
        SignalingStats {
            connections: self.connections.load(Ordering::SeqCst),
            disconnections: self.disconnections.load(Ordering::SeqCst),
            ..SignalingStats::default()
        }
    }
}

#[async_trait::async_trait]
impl SignalingClient for FakeSignalingClient {
    async fn connect(&self) -> NetworkResult<()> {
        self.connected.store(true, Ordering::SeqCst);
        self.connections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Connected);
        Ok(())
    }

    async fn disconnect(&self) -> NetworkResult<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.disconnections.fetch_add(1, Ordering::SeqCst);
        let _ = self.event_tx.send(SignalingEvent::Disconnected {
            reason: actr_runtime::wire::webrtc::DisconnectReason::Manual,
        });
        Ok(())
    }

    async fn send_register_request(
        &self,
        _request: RegisterRequest,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::NotImplemented(
            "register request not implemented in fake client".to_string(),
        ))
    }

    async fn send_unregister_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _reason: Option<String>,
    ) -> NetworkResult<UnregisterResponse> {
        Err(NetworkError::NotImplemented(
            "unregister request not implemented in fake client".to_string(),
        ))
    }

    async fn send_heartbeat(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _availability: actr_protocol::ServiceAvailabilityState,
        _power_reserve: f32,
        _mailbox_backlog: f32,
    ) -> NetworkResult<Pong> {
        Err(NetworkError::NotImplemented(
            "heartbeat not implemented in fake client".to_string(),
        ))
    }

    async fn send_route_candidates_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _request: RouteCandidatesRequest,
    ) -> NetworkResult<RouteCandidatesResponse> {
        Err(NetworkError::NotImplemented(
            "route candidates not implemented in fake client".to_string(),
        ))
    }

    async fn send_credential_update_request(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
    ) -> NetworkResult<RegisterResponse> {
        Err(NetworkError::NotImplemented(
            "credential update not implemented in fake client".to_string(),
        ))
    }

    async fn send_envelope(&self, _envelope: SignalingEnvelope) -> NetworkResult<()> {
        Err(NetworkError::NotImplemented(
            "send_envelope not implemented in fake client".to_string(),
        ))
    }

    async fn receive_envelope(&self) -> NetworkResult<Option<SignalingEnvelope>> {
        Err(NetworkError::NotImplemented(
            "receive_envelope not implemented in fake client".to_string(),
        ))
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn get_stats(&self) -> SignalingStats {
        self.stats()
    }

    fn subscribe_events(&self) -> broadcast::Receiver<SignalingEvent> {
        self.event_tx.subscribe()
    }

    async fn set_actor_id(&self, _actor_id: ActrId) {}

    async fn set_credential_state(&self, _credential_state: CredentialState) {}

    async fn clear_identity(&self) {}

    async fn get_signing_key(
        &self,
        _actor_id: ActrId,
        _credential: AIdCredential,
        _key_id: u32,
    ) -> NetworkResult<(u32, Vec<u8>)> {
        Err(NetworkError::NotImplemented(
            "get_signing_key not implemented in fake client".to_string(),
        ))
    }
}

#[tokio::test]
async fn test_network_available_debounced() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("first available should succeed");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.disconnections, 1);

    processor
        .process_network_available()
        .await
        .expect("second available should be debounced");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 2, "debounced call should not reconnect");
    assert_eq!(
        stats.disconnections, 1,
        "debounced call should not disconnect"
    );

    tokio::time::sleep(Duration::from_millis(600)).await;

    processor
        .process_network_available()
        .await
        .expect("available after window should succeed");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 3);
    assert_eq!(stats.disconnections, 2);
}

#[tokio::test]
async fn test_debounce_does_not_cross_event_types() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(500),
        },
    );

    processor
        .process_network_available()
        .await
        .expect("available should succeed");

    processor
        .process_network_lost()
        .await
        .expect("lost should not be debounced by available");

    let stats = client.get_stats();
    assert_eq!(stats.connections, 2);
    assert_eq!(stats.disconnections, 2);
}

/// 复现竞态条件：Swift 端同时发送 Network Available 和 Network Type Changed
///
/// 问题流程：
/// 1. T0: Swift 发送 Network Available 事件 -> Rust 处理，记录防抖时间戳
/// 2. T0+few ms: Swift 发送 Network Type Changed 事件 -> Rust 开始处理
/// 3. T0+670ms: TypeChanged 内部调用 process_network_available()
/// 4. 防抖检查：670ms < 2000ms (防抖窗口)，被过滤！
/// 5. 结果：WebSocket 断开后没有重连
///
/// 这个测试验证了这个设计缺陷
#[tokio::test]
async fn test_race_condition_type_changed_internal_call_debounced() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    // 使用较长的防抖窗口（模拟生产环境的 2 秒）
    let processor = Arc::new(DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    ));

    // 模拟 Swift 端同时发送两个事件
    // Event 1: Network Available (T0)
    tracing::info!("📱 [T0] Swift sends Network Available");
    processor
        .process_network_available()
        .await
        .expect("first available should succeed");

    let stats_after_available = client.get_stats();
    tracing::info!(
        "📊 After Available: connections={}, disconnections={}",
        stats_after_available.connections,
        stats_after_available.disconnections
    );

    // 断言：第一次 Available 应该成功执行
    // 初始 connect + process_available 中的 connect = 2
    assert_eq!(
        stats_after_available.connections, 2,
        "First Available should reconnect"
    );
    assert_eq!(
        stats_after_available.disconnections, 1,
        "First Available should disconnect once"
    );
    assert!(client.is_connected(), "Should be connected after Available");

    // Event 2: Network Type Changed (T0+10ms)
    // 模拟 Swift 几乎同时发送 TypeChanged 事件
    tokio::time::sleep(Duration::from_millis(10)).await;
    tracing::info!("📱 [T0+10ms] Swift sends Network Type Changed");

    // 开始处理 TypeChanged
    // 这会：
    // 1. 调用 process_network_lost() -> 断开连接
    // 2. 等待 500ms
    // 3. 调用 process_network_available() -> 被防抖过滤！！！

    processor
        .process_network_type_changed(true, false) // WiFi connected
        .await
        .expect("type changed should not return error");

    // 关键检查：TypeChanged 完成后的状态
    let stats_after_type_changed = client.get_stats();
    tracing::info!(
        "📊 After TypeChanged: connections={}, disconnections={}",
        stats_after_type_changed.connections,
        stats_after_type_changed.disconnections
    );
    tracing::info!("📊 Is connected: {}", client.is_connected());

    // 这是 BUG 的体现！
    // 期望行为：TypeChanged 应该重新连接 (connected = true)
    // 实际行为（由于防抖）：内部的 process_network_available 被过滤，没有重连

    // 让我们验证这个 BUG
    let is_connected_after = client.is_connected();
    let final_connections = stats_after_type_changed.connections;
    let final_disconnections = stats_after_type_changed.disconnections;

    // TypeChanged 会调用：
    // - process_network_lost() -> disconnections + 1
    // - process_network_available() -> 但被防抖！不会执行 connect
    //
    // 所以：
    // - disconnections 应该是 2 (Available断开1次 + TypeChanged调用Lost断开1次)
    // - connections 应该还是 2 (因为内部 Available 被防抖，没有执行 connect)
    // - is_connected 应该是 false！

    tracing::info!("🔍 Verifying race condition:");
    tracing::info!(
        "   - Final connections: {} (expected 2 due to debounce bug)",
        final_connections
    );
    tracing::info!(
        "   - Final disconnections: {} (expected 2)",
        final_disconnections
    );
    tracing::info!(
        "   - Is connected: {} (expected false due to bug)",
        is_connected_after
    );

    // 验证正确行为：reconnect_internal() 应该绕过防抖，成功重连
    //
    // TypeChanged 内部调用链：
    // 1. process_network_lost() -> disconnections + 1
    // 2. wait 500ms
    // 3. reconnect_internal() -> 绕过防抖，强制重连 -> connections + 1
    //
    // 因此期望：
    // - connections = 3 (initial + Available + TypeChanged内部重连)
    // - disconnections = 2 (Available断开 + TypeChanged断开)
    // - is_connected = true ✅ (成功重连)

    assert_eq!(
        final_connections, 3,
        "TypeChanged should trigger reconnect via reconnect_internal()"
    );
    assert_eq!(
        final_disconnections, 2,
        "TypeChanged should disconnect once, Available disconnects once"
    );
    assert!(
        is_connected_after,
        "BUG FIX VERIFIED: After TypeChanged, client should be connected because \
         reconnect_internal() bypasses debounce. \
         This proves the fix where internal calls correctly bypass debounce."
    );

    tracing::info!("✅ Debounce bypass working correctly!");
    tracing::info!("   reconnect_internal() successfully bypassed debounce");
    tracing::info!("   TypeChanged completed with successful reconnection");
}

/// 对比测试：当没有预先的 Available 事件时，TypeChanged 应该正常工作
#[tokio::test]
async fn test_type_changed_works_without_prior_available() {
    let client = Arc::new(FakeSignalingClient::new());
    client.connect().await.expect("initial connect");

    let processor = DefaultNetworkEventProcessor::new_with_debounce(
        client.clone(),
        None,
        DebounceConfig {
            window: Duration::from_millis(2000),
        },
    );

    // 直接发送 TypeChanged，没有预先的 Available
    processor
        .process_network_type_changed(true, false)
        .await
        .expect("type changed should succeed");

    let stats = client.get_stats();
    tracing::info!(
        "📊 TypeChanged without prior Available: connections={}, disconnections={}",
        stats.connections,
        stats.disconnections
    );

    // 这种情况下应该正常工作
    // TypeChanged 会：
    // 1. Lost: disconnect
    // 2. Wait 500ms
    // 3. Available: disconnect + connect
    //
    // 但是 Available 内部也会被 Lost 的防抖影响吗？让我们看看
    // 实际上 Available 和 Lost 是不同的事件类型，不共享防抖状态

    assert!(
        client.is_connected(),
        "Without prior Available event, TypeChanged should complete successfully"
    );
}
