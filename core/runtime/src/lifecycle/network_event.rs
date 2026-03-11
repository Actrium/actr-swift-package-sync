//! Network Event Handling Architecture
//!
//! This module defines the network event handling infrastructure.
//!
//! # Architecture Overview
//!
//! ```text
//!        ┌─────────────────────────────────────────────┐
//!        │ (FFI Path - Implemented)  (Actor Path - TODO)
//!        ▼                                             ▼
//! ┌──────────────────────────┐      ┌──────────────────────────┐
//! │ NetworkEventHandle       │      │ Direct Proto Message     │
//! │ • Platform FFI calls     │      │ • Actor call/tell        │
//! │ • Send via channel       │      │ • Send to actor mailbox  │
//! │ • Await result           │      │ • No handle needed       │
//! └────────┬─────────────────┘      └──────┬───────────────────┘
//!          │                               │
//!          └───────────────┬───────────────┘
//!                          │ Both trigger
//!                          ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  ActrNode::network_event_loop()                         │
//! │  • Receive event from channel (FFI path)                │
//! │  • Or handle message directly (Actor path - TODO)       │
//! │  • Delegate to NetworkEventProcessor                    │
//! │  • Send result back via channel                         │
//! └──────────────────────┬──────────────────────────────────┘
//!                        │ Delegate
//!                        ▼
//! ┌─────────────────────────────────────────────────────────┐
//! │  NetworkEventProcessor (Trait)                          │
//! │                                                          │
//! │  DefaultNetworkEventProcessor:                          │
//! │  • process_network_available()                          │
//! │    └─► Reconnect signaling + ICE restart                │
//! │  • process_network_lost()                               │
//! │    └─► Clear pending + disconnect                       │
//! │  • process_network_type_changed()                       │
//! │    └─► Disconnect + wait + reconnect                    │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Key Components
//!
//! - **NetworkEvent**: Event types (Available, Lost, TypeChanged)
//! - **NetworkEventResult**: Processing result with success/error/duration
//! - **NetworkEventProcessor**: Trait for custom event handling logic
//! - **DefaultNetworkEventProcessor**: Default implementation with signaling + WebRTC recovery
//!
//! # Usage Patterns
//!
//! ## 1. Platform FFI Call (Primary, Implemented)
//! ```ignore
//! // Platform layer calls NetworkEventHandle via FFI
//! let network_handle = system.create_network_event_handle();
//! let result = network_handle.handle_network_available().await?;
//! if result.success {
//!     println!("✅ Processed in {}ms", result.duration_ms);
//! }
//! ```
//!
//! ## 2. Actor Proto Message (Optional, TODO)
//! ```ignore
//! // TODO: actors send proto message directly (not yet implemented)
//! actor_ref.call(NetworkAvailableMessage).await?;
//! ```
//!
//! **Key Differences:**
//! - FFI path: Uses NetworkEventHandle + channel (implemented)
//! - Actor path: Direct proto message to mailbox (TODO, future enhancement)

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::wire::webrtc::{SignalingClient, coordinator::WebRtcCoordinator};

/// 网络事件类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NetworkEvent {
    /// 网络可用（从断网恢复）
    Available,

    /// 网络丢失（断网）
    Lost,

    /// 网络类型变化（WiFi ↔ Cellular）
    TypeChanged { is_wifi: bool, is_cellular: bool },

    /// 主动清理所有连接
    ///
    /// 用于应用生命周期管理场景：
    /// - 应用进入后台
    /// - 用户主动登出
    /// - 应用即将退出
    CleanupConnections,
}

/// 网络事件处理结果
#[derive(Debug, Clone)]
pub struct NetworkEventResult {
    /// 事件类型
    pub event: NetworkEvent,

    /// 处理是否成功
    pub success: bool,

    /// 错误信息（如果失败）
    pub error: Option<String>,

    /// 处理耗时（毫秒）
    pub duration_ms: u64,
}

impl NetworkEventResult {
    pub fn success(event: NetworkEvent, duration_ms: u64) -> Self {
        Self {
            event,
            success: true,
            error: None,
            duration_ms,
        }
    }

    pub fn failure(event: NetworkEvent, error: String, duration_ms: u64) -> Self {
        Self {
            event,
            success: false,
            error: Some(error),
            duration_ms,
        }
    }
}

/// 网络事件处理器 Trait
///
/// 定义网络事件的处理逻辑，可由用户自定义实现
#[async_trait::async_trait]
pub trait NetworkEventProcessor: Send + Sync {
    /// 处理网络可用事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_available(&self) -> Result<(), String>;

    /// 处理网络丢失事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_lost(&self) -> Result<(), String>;

    /// 处理网络类型变化事件
    ///
    /// # Returns
    /// - `Ok(())`: 处理成功
    /// - `Err(String)`: 处理失败，包含错误信息
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String>;

    /// 主动清理所有连接
    ///
    /// 此方法用于主动清理所有网络连接，适用于以下场景：
    /// - 应用进入后台（iOS/Android）
    /// - 用户主动登出
    /// - 应用即将退出
    /// - 需要重置网络状态
    ///
    /// # FFI Binding 说明
    ///
    /// 此方法专门设计用于 FFI binding，允许上层平台代码（Swift/Kotlin）
    /// 通过统一的 `NetworkEventProcessor` 接口主动管理连接生命周期。
    ///
    /// # 与事件响应的区别
    ///
    /// - `process_network_lost()`: 被动响应网络断开事件
    /// - `cleanup_connections()`: 主动清理连接（不依赖网络事件）
    ///
    /// # Returns
    /// - `Ok(())`: 清理成功
    /// - `Err(String)`: 清理失败，包含错误信息
    async fn cleanup_connections(&self) -> Result<(), String>;
}

/// 防抖配置
#[derive(Debug, Clone)]
pub struct DebounceConfig {
    /// 防抖时间窗口（同一事件在此时间内重复触发会被忽略）
    pub window: Duration,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            // 默认 1 秒防抖窗口
            window: Duration::from_secs(2),
        }
    }
}

/// 防抖状态跟踪
#[derive(Debug)]
struct DebounceState {
    last_available: tokio::sync::Mutex<Option<Instant>>,
    last_lost: tokio::sync::Mutex<Option<Instant>>,
    last_type_changed: tokio::sync::Mutex<Option<Instant>>,
}

impl DebounceState {
    fn new() -> Self {
        Self {
            last_available: tokio::sync::Mutex::new(None),
            last_lost: tokio::sync::Mutex::new(None),
            last_type_changed: tokio::sync::Mutex::new(None),
        }
    }
}

/// 默认网络事件处理器实现
pub struct DefaultNetworkEventProcessor {
    signaling_client: Arc<dyn SignalingClient>,
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    debounce_config: DebounceConfig,
    debounce_state: Arc<DebounceState>,
}

impl DefaultNetworkEventProcessor {
    pub fn new(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
    ) -> Self {
        Self::new_with_debounce(
            signaling_client,
            webrtc_coordinator,
            DebounceConfig::default(),
        )
    }

    pub fn new_with_debounce(
        signaling_client: Arc<dyn SignalingClient>,
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        debounce_config: DebounceConfig,
    ) -> Self {
        Self {
            signaling_client,
            webrtc_coordinator,
            debounce_config,
            debounce_state: Arc::new(DebounceState::new()),
        }
    }

    /// 检查事件是否应该被防抖过滤
    ///
    /// # Returns
    /// - `true`: 事件应该被处理
    /// - `false`: 事件在防抖窗口内，应该被忽略
    async fn should_process_event(&self, event: &NetworkEvent) -> bool {
        let now = Instant::now();

        match event {
            NetworkEvent::Available => {
                let mut last = self.debounce_state.last_available.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Available event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            NetworkEvent::Lost => {
                let mut last = self.debounce_state.last_lost.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network Lost event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            NetworkEvent::TypeChanged { .. } => {
                let mut last = self.debounce_state.last_type_changed.lock().await;
                if let Some(last_time) = *last {
                    if now.duration_since(last_time) < self.debounce_config.window {
                        tracing::debug!(
                            "⏸️  Debouncing Network TypeChanged event (last event was {:?} ago)",
                            now.duration_since(last_time)
                        );
                        return false;
                    }
                }
                *last = Some(now);
                true
            }
            // CleanupConnections 不进行防抖检查，主动清理总是立即执行
            NetworkEvent::CleanupConnections => {
                tracing::debug!(
                    "🧹 CleanupConnections event - no debouncing (always execute immediately)"
                );
                true
            }
        }
    }

    /// 内部重连方法（不进行防抖检查）
    ///
    /// 用于 `process_network_type_changed()` 等需要确保重连的场景
    /// 与 `process_network_available()` 的区别：
    /// - 不进行防抖检查（内部调用总是执行）
    /// - 适用于已经通过防抖检查的复合操作
    async fn reconnect_internal(&self) -> Result<(), String> {
        tracing::info!("🔄 Internal reconnect (bypassing debounce)");

        // Step 1: 强制断开现有连接（避免"僵尸连接"）
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting existing connection to ensure fresh state...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: 延迟等待网络稳定
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: 建立新的 WebSocket 连接
        tracing::info!("🔄 Reconnecting WebSocket...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ WebSocket reconnected successfully");
            }
            Err(e) => {
                let err_msg = format!("WebSocket reconnect failed: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        // Step 4: 触发 ICE 重启（如果 WebRTC 已初始化）
        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
impl NetworkEventProcessor for DefaultNetworkEventProcessor {
    /// 处理网络可用事件
    async fn process_network_available(&self) -> Result<(), String> {
        // 防抖检查
        if !self.should_process_event(&NetworkEvent::Available).await {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network available");

        // Step 1: 强制断开现有连接（避免"僵尸连接"）
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting existing connection to ensure fresh state...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: 延迟等待网络稳定
        // 注意：此延迟必须小于防抖窗口，否则会导致防抖失效
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: 建立新的 WebSocket 连接
        tracing::info!("🔄 Reconnecting WebSocket...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ WebSocket reconnected successfully");
            }
            Err(e) => {
                let err_msg = format!("WebSocket reconnect failed: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        // Step 4: 触发 ICE 重启（如果 WebRTC 已初始化）
        let coordinator = self.webrtc_coordinator.clone();

        if let Some(coordinator) = coordinator {
            tracing::info!("♻️ Triggering ICE restart for failed connections...");
            coordinator.retry_failed_connections().await;
        }

        Ok(())
    }

    /// 处理网络丢失事件
    async fn process_network_lost(&self) -> Result<(), String> {
        // 防抖检查
        if !self.should_process_event(&NetworkEvent::Lost).await {
            return Ok(());
        }

        tracing::info!("📱 Processing: Network lost");

        // Step 1: 清理待处理的 ICE 重启尝试
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        // Step 2: 主动断开 WebSocket
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        Ok(())
    }

    /// 处理网络类型变化事件
    async fn process_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<(), String> {
        // 防抖检查
        if !self
            .should_process_event(&NetworkEvent::TypeChanged {
                is_wifi,
                is_cellular,
            })
            .await
        {
            return Ok(());
        }

        tracing::info!(
            "📱 Processing: Network type changed (WiFi={}, Cellular={})",
            is_wifi,
            is_cellular
        );

        // 网络类型变化通常意味着 IP 地址变化
        // 视为断网 + 恢复序列

        // Step 1: 清理现有连接
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("🧹 Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;
        }

        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            let _ = self.signaling_client.disconnect().await;
        }

        // Step 2: 等待网络稳定
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Step 3: 使用内部重连方法（绕过防抖检查）
        self.reconnect_internal().await?;

        Ok(())
    }

    /// 主动清理所有连接
    ///
    /// 与 `process_network_lost()` 的区别：
    /// - 不进行防抖检查（主动调用总是执行）
    /// - 适用于应用生命周期管理，而非网络事件响应
    async fn cleanup_connections(&self) -> Result<(), String> {
        tracing::info!("🧹 Manually cleaning up all connections...");

        // Step 1: 清理待处理的 ICE 重启尝试
        if let Some(ref coordinator) = self.webrtc_coordinator {
            tracing::info!("♻️  Clearing pending ICE restart attempts...");
            coordinator.clear_pending_restarts().await;

            // Step 2: 关闭所有 WebRTC peer connections
            tracing::info!("🔻 Closing all WebRTC peer connections...");
            if let Err(e) = coordinator.close_all_peers().await {
                let err_msg = format!("Failed to close all peers: {}", e);
                tracing::warn!("⚠️  {}", err_msg);
                // 不返回错误，继续清理其他资源
            } else {
                tracing::info!("✅ All WebRTC peer connections closed");
            }
        }

        // Step 3: 主动断开 WebSocket
        if self.signaling_client.is_connected() {
            tracing::info!("🔌 Disconnecting WebSocket...");
            match self.signaling_client.disconnect().await {
                Ok(_) => {
                    tracing::info!("✅ WebSocket disconnected successfully");
                }
                Err(e) => {
                    let err_msg = format!("Failed to disconnect WebSocket: {}", e);
                    tracing::warn!("⚠️  {}", err_msg);
                    // 不返回错误，继续清理其他资源
                }
            }
        }

        tracing::info!("✅ Connection cleanup completed");

        // Step 4: 立即重新建立信令连接
        // 确保 App 回到前台后立即可用，不需要等待自动重连
        tracing::info!("🔌 Re-establishing signaling connection...");
        match self.signaling_client.connect().await {
            Ok(_) => {
                tracing::info!("✅ Signaling reconnected successfully after cleanup");
            }
            Err(e) => {
                let err_msg = format!("Failed to reconnect signaling after cleanup: {}", e);
                tracing::error!("❌ {}", err_msg);
                return Err(err_msg);
            }
        }

        tracing::info!("✅ Connection cleanup and reconnect completed");
        Ok(())
    }
}

/// Network Event Handle
///
/// Lightweight handle for sending network events and receiving processing results.
/// Created by `ActrSystem::create_network_event_handle()`.
pub struct NetworkEventHandle {
    /// Event sender (to ActrNode)
    event_tx: tokio::sync::mpsc::Sender<NetworkEvent>,

    /// Result receiver (from ActrNode)
    /// Wrapped in Arc<Mutex> to allow cloning
    result_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<NetworkEventResult>>>,
}

impl NetworkEventHandle {
    /// Create a new NetworkEventHandle
    pub fn new(
        event_tx: tokio::sync::mpsc::Sender<NetworkEvent>,
        result_rx: tokio::sync::mpsc::Receiver<NetworkEventResult>,
    ) -> Self {
        Self {
            event_tx,
            result_rx: Arc::new(tokio::sync::Mutex::new(result_rx)),
        }
    }

    /// Handle network available event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_available(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::Available)
            .await
    }

    /// Handle network lost event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_lost(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::Lost).await
    }

    /// Handle network type changed event
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn handle_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::TypeChanged {
            is_wifi,
            is_cellular,
        })
        .await
    }

    /// 主动清理所有连接
    ///
    /// 此方法用于主动清理所有网络连接，适用于以下场景：
    /// - 应用进入后台（iOS/Android）
    /// - 用户主动登出
    /// - 应用即将退出
    /// - 需要重置网络状态
    ///
    /// # Returns
    /// - `Ok(NetworkEventResult)`: Processing result
    /// - `Err(String)`: Failed to send event or receive result
    pub async fn cleanup_connections(&self) -> Result<NetworkEventResult, String> {
        self.send_event_and_await_result(NetworkEvent::CleanupConnections)
            .await
    }

    /// Send event and await result (internal helper)
    async fn send_event_and_await_result(
        &self,
        event: NetworkEvent,
    ) -> Result<NetworkEventResult, String> {
        // Send event
        self.event_tx
            .send(event.clone())
            .await
            .map_err(|e| format!("Failed to send network event: {}", e))?;

        // Await result
        let mut rx = self.result_rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| "Failed to receive network event result".to_string())
    }
}

impl Clone for NetworkEventHandle {
    fn clone(&self) -> Self {
        Self {
            event_tx: self.event_tx.clone(),
            result_rx: self.result_rx.clone(),
        }
    }
}

/// 去重网络事件：按类型去重，保留每种类型的最新事件，并保持原始顺序
///
/// # 算法
///
/// 1. 遍历所有事件，按类型分组
/// 2. 对于每种类型，只保留最后出现的事件（最新的）
/// 3. 按原始索引排序，保持事件的时序关系
///
/// # 为什么需要去重？
///
/// 在后台恢复或网络频繁变化时，事件队列可能积压大量事件。
/// 对于网络状态事件，只有最新的状态才有意义，旧的状态已经过时。
/// 但是不同类型的事件代表不同的状态变化，都需要保留。
///
/// - `[Available, Lost, Available]` → 如果只保留最后的 `Available`，会丢失中间的 `Lost`
/// - `[Lost, Available, Lost]` → 如果只保留最后的 `Lost`，会丢失中间的 `Available`
///
/// 按类型去重可以确保每种状态变化都被处理。
pub fn deduplicate_network_events(events: Vec<NetworkEvent>) -> Vec<NetworkEvent> {
    use std::collections::HashMap;
    use std::mem::discriminant;

    if events.is_empty() {
        return vec![];
    }

    // 按类型分组，保留每种类型的最新事件（最大索引）
    // 使用 discriminant 作为 key，可以区分枚举的不同变体
    let mut latest_by_type: HashMap<std::mem::Discriminant<NetworkEvent>, (usize, NetworkEvent)> =
        HashMap::new();

    for (index, event) in events.into_iter().enumerate() {
        let event_discriminant = discriminant(&event);
        latest_by_type
            .entry(event_discriminant)
            .and_modify(|(idx, e)| {
                // 更新为更新的事件
                *idx = index;
                *e = event.clone();
            })
            .or_insert((index, event));
    }

    // 按原始索引排序，保持事件的时序关系
    let mut deduplicated: Vec<_> = latest_by_type.into_values().collect();
    deduplicated.sort_by_key(|(index, _)| *index);

    // 提取事件
    deduplicated.into_iter().map(|(_, event)| event).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplicate_empty() {
        let events = vec![];
        let result = deduplicate_network_events(events);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_deduplicate_single_event() {
        let events = vec![NetworkEvent::Available];
        let result = deduplicate_network_events(events);
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_same_type_events() {
        let events = vec![
            NetworkEvent::Available,
            NetworkEvent::Available,
            NetworkEvent::Available,
        ];
        let result = deduplicate_network_events(events);

        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_different_type_events() {
        let events = vec![
            NetworkEvent::Available,
            NetworkEvent::Lost,
            NetworkEvent::TypeChanged {
                is_wifi: true,
                is_cellular: false,
            },
        ];
        let result = deduplicate_network_events(events);

        assert_eq!(result.len(), 3);
        assert!(matches!(result[0], NetworkEvent::Available));
        assert!(matches!(result[1], NetworkEvent::Lost));
        assert!(matches!(result[2], NetworkEvent::TypeChanged { .. }));
    }

    #[test]
    fn test_deduplicate_mixed_events() {
        let events = vec![
            NetworkEvent::Available, // #0
            NetworkEvent::Lost,      // #1
            NetworkEvent::Available, // #2 (覆盖 #0)
            NetworkEvent::TypeChanged {
                // #3
                is_wifi: true,
                is_cellular: false,
            },
            NetworkEvent::Lost,      // #4 (覆盖 #1)
            NetworkEvent::Available, // #5 (覆盖 #2)
        ];
        let result = deduplicate_network_events(events);

        // 应该保留：TypeChanged (#3), Lost (#4), Available (#5)
        assert_eq!(result.len(), 3);

        // 验证顺序（按原始索引排序）
        assert!(matches!(result[0], NetworkEvent::TypeChanged { .. }));
        assert!(matches!(result[1], NetworkEvent::Lost));
        assert!(matches!(result[2], NetworkEvent::Available));
    }

    #[test]
    fn test_deduplicate_preserves_order() {
        let events = vec![
            NetworkEvent::Lost, // #0
            NetworkEvent::TypeChanged {
                // #1
                is_wifi: true,
                is_cellular: false,
            },
            NetworkEvent::Available, // #2
            NetworkEvent::Lost,      // #3 (覆盖 #0)
        ];
        let result = deduplicate_network_events(events);

        // 应该保留：TypeChanged (#1), Available (#2), Lost (#3)
        assert_eq!(result.len(), 3);

        // 验证顺序
        assert!(matches!(result[0], NetworkEvent::TypeChanged { .. }));
        assert!(matches!(result[1], NetworkEvent::Available));
        assert!(matches!(result[2], NetworkEvent::Lost));
    }
}
