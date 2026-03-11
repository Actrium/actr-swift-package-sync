//! WireBuilder - Wire layer component builder
//!
//! Provides default Wire component builder implementation, supporting:
//! - WebRTC P2P connections (through WebRtcCoordinator)
//! - WebSocket C/S connections
//! - CancellationToken for terminating in-progress connection creation

use super::Dest; // Re-exported from actr-framework
use super::error::{NetworkError, NetworkResult};
use super::manager::WireBuilder;
use super::wire_handle::WireHandle;
use crate::lifecycle::CredentialState;
use crate::wire::webrtc::WebRtcCoordinator;
use crate::wire::websocket::WebSocketConnection;
use actr_protocol::ActrId;
use actr_protocol::prost::Message as ProstMessage;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Default Wire builder configuration
pub struct DefaultWireBuilderConfig {
    /// 本地节点身份（hex-encoded protobuf ActrId bytes），出站 WebSocket 握手时随 X-Actr-Source-ID 头发送
    pub local_id_hex: String,

    /// Enable WebRTC
    pub enable_webrtc: bool,

    /// Enable WebSocket
    pub enable_websocket: bool,

    /// Shared map of discovered WebSocket direct-connect URLs, keyed by ActrId.
    ///
    /// Populated by `ActrNode::discover_route_candidates` after receiving ws_address info
    /// from the signaling server.  When a connection to an ActrId is needed and this map
    /// contains an entry for it, the stored URL is used instead of the url_template.
    pub discovered_ws_addresses: Arc<RwLock<HashMap<ActrId, String>>>,

    /// 本地节点凭证状态（可选）。出站 WebSocket 握手时将当前 credential 以 base64 编码随
    /// X-Actr-Credential 头发送，供对端进行 Ed25519 签名验证。
    pub credential_state: Option<CredentialState>,
}

impl Default for DefaultWireBuilderConfig {
    fn default() -> Self {
        Self {
            local_id_hex: String::new(),
            enable_webrtc: true,
            enable_websocket: true,
            discovered_ws_addresses: Arc::new(RwLock::new(HashMap::new())),
            credential_state: None,
        }
    }
}

/// default Wire construct build device
///
/// based onconfigurationCreate WebRTC and/or WebSocket Wire group file 。
/// Supportsaturatedand format Connect（ same temporal attempt try multiple typeConnectType）。
pub struct DefaultWireBuilder {
    /// WebRTC coordinator（optional）
    webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,

    /// 本地节点身份 hex（出站 WS 握手时作为 X-Actr-Source-ID 发送）
    local_id_hex: String,

    /// Shared map of discovered WebSocket URLs (from signaling discovery)
    discovered_ws_addresses: Arc<RwLock<HashMap<ActrId, String>>>,

    /// 本地节点凭证状态（出站 WS 握手时提供 X-Actr-Credential 供对端验签）
    credential_state: Option<CredentialState>,

    /// configuration
    config: DefaultWireBuilderConfig,
}

impl DefaultWireBuilder {
    /// Create new Wire construct build device
    ///
    /// # Arguments
    /// - `webrtc_coordinator`: WebRTC coordinator（If start usage WebRTC）
    /// - `config`: construct build device configuration
    pub fn new(
        webrtc_coordinator: Option<Arc<WebRtcCoordinator>>,
        config: DefaultWireBuilderConfig,
    ) -> Self {
        Self {
            webrtc_coordinator,
            local_id_hex: config.local_id_hex.clone(),
            discovered_ws_addresses: config.discovered_ws_addresses.clone(),
            credential_state: config.credential_state.clone(),
            config,
        }
    }

    /// 查询目标节点的 WebSocket 直连 URL（仅来自服务发现）
    async fn resolve_websocket_url(&self, dest: &Dest) -> Option<String> {
        if let Dest::Actor(actor_id) = dest {
            let map = self.discovered_ws_addresses.read().await;
            if let Some(url) = map.get(actor_id) {
                tracing::debug!(
                    "🔎 [Factory] Using discovered WebSocket URL for {}: {}",
                    actor_id.serial_number,
                    url
                );
                return Some(url.clone());
            }
        }
        None
    }
}

#[async_trait]
impl WireBuilder for DefaultWireBuilder {
    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections(&self, dest: &Dest) -> NetworkResult<Vec<WireHandle>> {
        // Delegate to method with no cancel token
        self.create_connections_with_cancel(dest, None).await
    }

    #[cfg_attr(feature = "opentelemetry", tracing::instrument(skip_all))]
    async fn create_connections_with_cancel(
        &self,
        dest: &Dest,
        cancel_token: Option<CancellationToken>,
    ) -> NetworkResult<Vec<WireHandle>> {
        let mut connections = Vec::new();

        // Helper to check cancellation
        let check_cancelled = |token: &Option<CancellationToken>| -> NetworkResult<()> {
            if let Some(t) = token {
                if t.is_cancelled() {
                    return Err(NetworkError::ConnectionClosed(
                        "Connection creation cancelled".to_string(),
                    ));
                }
            }
            Ok(())
        };

        // 1. Check if already cancelled
        check_cancelled(&cancel_token)?;

        // 2. 尝试建立 WebSocket 连接
        // URL 来自服务发现（discovered_ws_addresses）。若未发现，则本次跳过 WebSocket 连接。
        if self.config.enable_websocket {
            check_cancelled(&cancel_token)?;

            if let Some(url) = self.resolve_websocket_url(dest).await {
                tracing::debug!("🏭 [Factory] Create WebSocket Connect: {}", url);
                let mut ws_conn =
                    WebSocketConnection::new(url).with_local_id(self.local_id_hex.clone());

                // 携带本地 credential，供对端 WebSocketGate 进行 Ed25519 验签
                if let Some(ref cred_state) = self.credential_state {
                    let credential = cred_state.credential().await;
                    let cred_bytes = credential.encode_to_vec();
                    use base64::Engine as _;
                    let cred_b64 = base64::engine::general_purpose::STANDARD.encode(&cred_bytes);
                    ws_conn = ws_conn.with_credential_b64(cred_b64);
                }

                connections.push(WireHandle::WebSocket(ws_conn));
            } else {
                tracing::debug!(
                    "🔎 [Factory] No WebSocket URL available for {:?}, skipping WS connection",
                    dest
                );
            }
        }

        // 3. Check cancellation before WebRTC
        check_cancelled(&cancel_token)?;

        // 4. attempt try Create WebRTC Connect
        if self.config.enable_webrtc {
            if let Some(coordinator) = &self.webrtc_coordinator {
                // WebRTC merely Support Actor Type
                if dest.is_actor() {
                    tracing::debug!("🏭 [Factory] Create WebRTC Connectto: {:?}", dest);

                    // Check cancellation before long-running operation
                    check_cancelled(&cancel_token)?;

                    match coordinator
                        .create_connection(dest, cancel_token.clone())
                        .await
                    {
                        Ok(webrtc_conn) => {
                            // Check cancellation after creation
                            if let Err(e) = check_cancelled(&cancel_token) {
                                // Clean up newly created connection
                                if let Err(close_err) = webrtc_conn.close().await {
                                    tracing::warn!(
                                        "⚠️ [Factory] Failed to close cancelled connection: {}",
                                        close_err
                                    );
                                }
                                return Err(e);
                            }
                            connections.push(WireHandle::WebRTC(webrtc_conn));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "❌ [Factory] WebRTC ConnectCreatefailure: {:?}: {}",
                                dest,
                                e
                            );
                            // not ReturnsError，allowusingotherConnectType
                        }
                    }
                } else {
                    tracing::debug!(
                        "ℹ️ [Factory] WebRTC not Support Shell item mark ，skip through "
                    );
                }
            } else {
                tracing::warn!("⚠️ [Factory] WebRTC enabled but not Provide WebRtcCoordinator");
            }
        }

        tracing::info!(
            "✨ [Factory] as {:?} Create done {} Connect",
            dest,
            connections.len()
        );

        Ok(connections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::ActrId;

    #[tokio::test]
    async fn test_no_ws_connection_without_discovery() {
        // WebSocket URL 仅来自服务发现；无发现记录时不应建立 WS 连接
        let config = DefaultWireBuilderConfig {
            enable_websocket: true,
            enable_webrtc: false,
            local_id_hex: "deadbeef".to_string(),
            discovered_ws_addresses: Arc::new(RwLock::new(HashMap::new())),
            credential_state: None,
        };
        let factory = DefaultWireBuilder::new(None, config);
        let dest = Dest::actor(ActrId::default());
        let connections = factory.create_connections(&dest).await.unwrap();
        assert!(connections.is_empty());
    }

    #[tokio::test]
    async fn test_ws_connection_from_discovery() {
        // 服务发现写入地址后，应能建立 WS 连接
        let map = Arc::new(RwLock::new(HashMap::new()));
        let actor_id = ActrId::default();
        map.write()
            .await
            .insert(actor_id.clone(), "ws://localhost:9001".to_string());

        let config = DefaultWireBuilderConfig {
            enable_websocket: true,
            enable_webrtc: false,
            local_id_hex: "deadbeef".to_string(),
            discovered_ws_addresses: map,
            credential_state: None,
        };
        let factory = DefaultWireBuilder::new(None, config);
        let dest = Dest::actor(actor_id);
        let connections = factory.create_connections(&dest).await.unwrap();
        assert_eq!(connections.len(), 1);
        assert!(matches!(connections[0], WireHandle::WebSocket(_)));
    }
}
