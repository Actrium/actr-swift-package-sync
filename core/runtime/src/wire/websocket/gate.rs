//! WebSocketGate - WebSocket 入站连接的消息路由器
//!
//! 从 `WebSocketServer` 通道接收新建连接（含发送方 ActrId bytes 和 AIdCredential），
//! 对每个连接先进行 Ed25519 credential 验签（若已配置 `WsAuthContext`），
//! 验签通过后按 PayloadType 路由消息到 Mailbox 或 DataStreamRegistry。
//!
//! # 设计对比 WebRtcGate
//!
//! | 关注点 | WebRtcGate | WebSocketGate |
//! |--------|-----------|---------------|
//! | 传输层 | WebRTC DataChannel | WebSocket (TCP) |
//! | 发送方认证 | actrix 信令验证 credential | 本地 Ed25519 验签（AisKeyCache） |
//! | 消息聚合 | `WebRtcCoordinator.receive_message()` | 逐连接读取 `DataLane` |

use super::connection::WebSocketConnection;
use super::server::InboundWsConn;
use crate::ais_key_cache::AisKeyCache;
use crate::error::{ActorResult, ActrError};
use crate::inbound::DataStreamRegistry;
use crate::lifecycle::CredentialState;
use crate::wire::webrtc::SignalingClient;
use actr_framework::Bytes;
use actr_protocol::prost::Message as ProstMessage;
use actr_protocol::{
    AIdCredential, ActrId, ActrIdExt, DataStream, IdentityClaims, PayloadType, RpcEnvelope,
};
use actr_runtime_mailbox::{Mailbox, MessagePriority};
use ed25519_dalek::{Signature, Verifier as Ed25519Verifier};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};

/// Pending requests map type: request_id → (target_actor_id, oneshot response sender)
type PendingRequestsMap =
    Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>;

/// WebSocket 身份验证上下文（可选）
///
/// 配置后，gate 将对每个入站连接执行 Ed25519 credential 验签：
/// - 验签失败的连接直接丢弃，不启动 lane reader
/// - 连接方未携带 credential 时，视同验签失败
pub struct WsAuthContext {
    /// AIS signing 公钥缓存（本地命中直接验签，miss 时通过 signaling 拉取）
    pub ais_key_cache: Arc<AisKeyCache>,
    /// 本机 ActrId（cache miss 时向 signaling 请求公钥所需）
    pub actor_id: ActrId,
    /// 本机凭证状态（cache miss 时向 signaling 认证所需）
    pub credential_state: CredentialState,
    /// Signaling 客户端（cache miss 时拉取公钥）
    pub signaling_client: Arc<dyn SignalingClient>,
}

/// WebSocketGate - 接收并路由入站 WebSocket 消息
pub struct WebSocketGate {
    /// 入站连接通道（take 一次后 move 进 background task）
    conn_rx: tokio::sync::Mutex<Option<mpsc::Receiver<InboundWsConn>>>,

    /// 待响应请求表（request_id → (caller_id, oneshot::Sender)）
    /// **与 OutprocOutGate 共享**，以便正确路由 Response
    pending_requests: PendingRequestsMap,

    /// DataStream 注册表（fast-path 流消息路由）
    data_stream_registry: Arc<DataStreamRegistry>,

    /// 入站连接身份验证上下文
    auth_ctx: Option<Arc<WsAuthContext>>,
}

impl WebSocketGate {
    /// 创建 WebSocketGate
    ///
    /// # Arguments
    /// - `conn_rx`: 来自 `WebSocketServer::bind()` 的接收端
    /// - `pending_requests`: 与 OutprocOutGate 共享的待响应表
    /// - `data_stream_registry`: DataStream 注册表
    /// - `auth_ctx`: 身份验证上下文（配置后对所有入站连接强制验签）
    pub fn new(
        conn_rx: mpsc::Receiver<InboundWsConn>,
        pending_requests: PendingRequestsMap,
        data_stream_registry: Arc<DataStreamRegistry>,
        auth_ctx: Option<WsAuthContext>,
    ) -> Self {
        Self {
            conn_rx: tokio::sync::Mutex::new(Some(conn_rx)),
            pending_requests,
            data_stream_registry,
            auth_ctx: auth_ctx.map(Arc::new),
        }
    }

    /// 处理 RpcEnvelope：Response 唤醒等待方，Request 入队 Mailbox
    async fn handle_envelope(
        envelope: RpcEnvelope,
        from_bytes: Vec<u8>,
        data: Bytes,
        payload_type: PayloadType,
        pending_requests: PendingRequestsMap,
        mailbox: Arc<dyn Mailbox>,
    ) {
        let request_id = envelope.request_id.clone();

        let mut pending = pending_requests.write().await;
        if let Some((target, response_tx)) = pending.remove(&request_id) {
            drop(pending);
            tracing::debug!(
                "📬 WS Received RPC Response: request_id={}, target={}",
                request_id,
                target.to_string_repr()
            );

            let result = match (envelope.payload, envelope.error) {
                (Some(payload), None) => Ok(payload),
                (None, Some(error)) => Err(ActrError::Unavailable(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                ))),
                _ => Err(ActrError::DecodeFailure(
                    "Invalid RpcEnvelope: payload and error fields inconsistent".to_string(),
                )),
            };
            let _ = response_tx.send(result);
        } else {
            drop(pending);
            tracing::debug!("📥 WS Received RPC Request: request_id={}", request_id);

            let priority = match payload_type {
                PayloadType::RpcSignal => MessagePriority::High,
                _ => MessagePriority::Normal,
            };

            match mailbox.enqueue(from_bytes, data.to_vec(), priority).await {
                Ok(msg_id) => {
                    tracing::debug!(
                        "✅ WS RPC message enqueued: msg_id={}, priority={:?}",
                        msg_id,
                        priority
                    );
                }
                Err(e) => {
                    tracing::error!("❌ WS Mailbox enqueue failed: {:?}", e);
                }
            }
        }
    }

    /// 验证入站连接的 AIdCredential Ed25519 签名
    ///
    /// 返回 `Some(verified_actor_id_str)` 表示验签通过，`None` 表示失败（已记录日志）。
    /// `source_id_bytes` 为 `X-Actr-Source-ID` 提供的 ActrId protobuf bytes。
    async fn verify_credential(
        credential: &AIdCredential,
        source_id_bytes: &[u8],
        auth_ctx: &WsAuthContext,
    ) -> Option<()> {
        // 取得 actor B 自身的 ActrId 和 credential（用于 cache miss 拉取公钥时向 signaling 认证）
        let local_credential = auth_ctx.credential_state.credential().await;

        // 从 AisKeyCache 获取 key_id 对应的 verifying key（本地命中或从 signaling 拉取）
        let verifying_key = match auth_ctx
            .ais_key_cache
            .get_or_fetch(
                credential.key_id,
                &auth_ctx.actor_id,
                &local_credential,
                auth_ctx.signaling_client.as_ref(),
            )
            .await
        {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(
                    key_id = credential.key_id,
                    error = ?e,
                    "⚠️ WS credential 验签失败：无法获取 signing key"
                );
                return None;
            }
        };

        // Ed25519 验签
        let sig_result =
            credential.signature[..]
                .try_into()
                .ok()
                .and_then(|sig_bytes: [u8; 64]| {
                    let signature = Signature::from_bytes(&sig_bytes);
                    verifying_key
                        .verify(&credential.claims[..], &signature)
                        .ok()
                });
        if sig_result.is_none() {
            tracing::warn!(
                key_id = credential.key_id,
                "⚠️ WS AIdCredential Ed25519 验签失败"
            );
            return None;
        }

        // 解码 IdentityClaims
        let claims = match IdentityClaims::decode(&credential.claims[..]) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key_id = credential.key_id, error = ?e, "⚠️ WS IdentityClaims proto 解码失败");
                return None;
            }
        };

        // 检查 expires_at
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if claims.expires_at <= now {
            tracing::warn!(
                key_id = credential.key_id,
                expires_at = claims.expires_at,
                "⚠️ WS AIdCredential 已过期"
            );
            return None;
        }

        // 校验 claims.actor_id 与 X-Actr-Source-ID 一致（防止身份声称不一致）
        match ActrId::decode(source_id_bytes) {
            Ok(source_actor_id) => {
                let source_repr = source_actor_id.to_string_repr();
                if claims.actor_id != source_repr {
                    tracing::warn!(
                        claimed = %claims.actor_id,
                        source_id = %source_repr,
                        "⚠️ WS credential actor_id 与 X-Actr-Source-ID 不一致，拒绝连接"
                    );
                    return None;
                }
                tracing::info!(
                    actor_id = %claims.actor_id,
                    "✅ WS 入站连接身份验证通过"
                );
            }
            Err(e) => {
                tracing::warn!(error = ?e, "⚠️ WS X-Actr-Source-ID 解码失败，拒绝连接");
                return None;
            }
        }

        Some(())
    }

    /// 为单个 WebSocket 连接启动接收任务
    ///
    /// 逐一读取 `PayloadType::RpcReliable`、`RpcSignal`、`StreamReliable`、
    /// `StreamLatencyFirst` 四条 lane，对每条 lane 起一个独立 task。
    fn spawn_connection_tasks(
        conn: WebSocketConnection,
        source_id: Vec<u8>,
        pending_requests: PendingRequestsMap,
        data_stream_registry: Arc<DataStreamRegistry>,
        mailbox: Arc<dyn Mailbox>,
    ) {
        // Spawn per-PayloadType receive tasks
        for pt in [
            PayloadType::RpcReliable,
            PayloadType::RpcSignal,
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
        ] {
            let conn_clone = conn.clone();
            let src = source_id.clone();
            let pending = pending_requests.clone();
            let registry = data_stream_registry.clone();
            let mb = mailbox.clone();

            tokio::spawn(async move {
                // get_lane lazily creates the mpsc channel and registers in router
                let lane = match conn_clone.get_lane(pt).await {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("❌ WS get_lane({:?}) failed: {:?}", pt, e);
                        return;
                    }
                };

                tracing::debug!("📡 WS lane reader started for {:?}", pt);

                loop {
                    match lane.recv().await {
                        Ok(data) => {
                            let data_bytes = Bytes::copy_from_slice(&data);

                            match pt {
                                PayloadType::RpcReliable | PayloadType::RpcSignal => {
                                    match RpcEnvelope::decode(&data[..]) {
                                        Ok(envelope) => {
                                            Self::handle_envelope(
                                                envelope,
                                                src.clone(),
                                                data_bytes,
                                                pt,
                                                pending.clone(),
                                                mb.clone(),
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "❌ WS Failed to decode RpcEnvelope: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                                PayloadType::StreamReliable | PayloadType::StreamLatencyFirst => {
                                    match DataStream::decode(&data[..]) {
                                        Ok(chunk) => {
                                            tracing::debug!(
                                                "📦 WS Received DataStream: stream_id={}, seq={}",
                                                chunk.stream_id,
                                                chunk.sequence,
                                            );
                                            match ActrId::decode(&src[..]) {
                                                Ok(sender_id) => {
                                                    registry.dispatch(chunk, sender_id).await;
                                                }
                                                Err(e) => {
                                                    tracing::error!(
                                                        "❌ WS Failed to decode sender ActrId: {:?}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "❌ WS Failed to decode DataStream: {:?}",
                                                e
                                            );
                                        }
                                    }
                                }
                                PayloadType::MediaRtp => {
                                    tracing::warn!(
                                        "⚠️ MediaRtp received in WebSocketGate (unexpected)"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::info!("🔌 WS lane {:?} closed: {:?}", pt, e);
                            break;
                        }
                    }
                }

                tracing::debug!("📡 WS lane reader exited for {:?}", pt);
            });
        }
    }

    /// 启动连接接受循环（由 ActrNode 调用，只能调用一次）
    ///
    /// 内部 take 出 `conn_rx`，move 进 background task 以无锁接收新连接。
    /// 若配置了 `auth_ctx`，则对每个入站连接先进行 credential 验签，
    /// 验签失败则丢弃连接；验签通过后调用 `spawn_connection_tasks`。
    pub async fn start_receive_loop(&self, mailbox: Arc<dyn Mailbox>) -> ActorResult<()> {
        let rx = self.conn_rx.lock().await.take().ok_or_else(|| {
            ActrError::Internal("WebSocketGate: start_receive_loop already called".to_string())
        })?;

        let pending_requests = self.pending_requests.clone();
        let data_stream_registry = self.data_stream_registry.clone();
        let auth_ctx = self.auth_ctx.clone();

        tokio::spawn(async move {
            tracing::info!("🚀 WebSocketGate receive loop started");

            let mut rx = rx;
            while let Some((conn, source_id, credential_opt)) = rx.recv().await {
                tracing::info!(
                    "🔗 WS new inbound connection (source_id len={}, has_credential={})",
                    source_id.len(),
                    credential_opt.is_some()
                );

                // Credential 验签（若已配置 auth_ctx）
                if let Some(ref ctx) = auth_ctx {
                    match credential_opt {
                        Some(ref credential) => {
                            if Self::verify_credential(credential, &source_id, ctx)
                                .await
                                .is_none()
                            {
                                tracing::warn!("⚠️ WS 入站连接 credential 验签失败，丢弃连接");
                                continue; // 丢弃连接，继续等待下一个
                            }
                        }
                        None => {
                            tracing::warn!(
                                "⚠️ WS 入站连接未携带 X-Actr-Credential，拒绝连接（auth_ctx 已配置）"
                            );
                            continue;
                        }
                    }

                    Self::spawn_connection_tasks(
                        conn,
                        source_id,
                        pending_requests.clone(),
                        data_stream_registry.clone(),
                        mailbox.clone(),
                    );
                } else {
                    tracing::error!("❌ WS auth_ctx 未配置，拒绝连接（配置错误）");
                }
            }

            tracing::info!("🔌 WebSocketGate receive loop exited");
        });

        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrIdExt, ActrType, IdentityClaims, Realm};
    use actr_runtime_mailbox::{MailboxStats, MessageRecord, StorageResult};
    use async_trait::async_trait;
    use ed25519_dalek::{Signer, SigningKey};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::mpsc;
    use uuid::Uuid;

    // ─── helpers ─────────────────────────────────────────────────────────────

    fn test_actor_id(serial: u64) -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: serial,
            r#type: ActrType {
                manufacturer: "test".to_string(),
                name: "node".to_string(),
                version: "v1".to_string(),
            },
        }
    }

    /// 用固定种子生成可重复的 Ed25519 密钥对
    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// 构造一个能通过验签的完整 AIdCredential
    fn make_valid_credential(
        sk: &SigningKey,
        actor_id: &ActrId,
        expires_at: u64,
        key_id: u32,
    ) -> AIdCredential {
        let claims = IdentityClaims {
            actor_id: actor_id.to_string_repr(),
            expires_at,
            realm_id: actor_id.realm.realm_id,
        };
        let claims_bytes = actr_protocol::prost::Message::encode_to_vec(&claims);
        let signature = sk.sign(&claims_bytes);
        AIdCredential {
            key_id,
            claims: claims_bytes.into(),
            signature: signature.to_bytes().to_vec().into(),
        }
    }

    fn future_ts() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600
    }

    fn past_ts() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 1
    }

    // ─── mock SignalingClient (同 ais_key_cache tests，内联以避免模块依赖) ──

    struct NullSignaling;

    #[async_trait]
    impl crate::wire::SignalingClient for NullSignaling {
        async fn connect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        async fn disconnect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            false
        }
        fn get_stats(&self) -> crate::wire::webrtc::SignalingStats {
            Default::default()
        }
        fn subscribe_events(
            &self,
        ) -> tokio::sync::broadcast::Receiver<crate::wire::webrtc::SignalingEvent> {
            tokio::sync::broadcast::channel(1).1
        }
        async fn set_actor_id(&self, _: ActrId) {}
        async fn set_credential_state(&self, _: crate::lifecycle::CredentialState) {}
        async fn clear_identity(&self) {}
        async fn send_register_request(
            &self,
            _: actr_protocol::RegisterRequest,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RegisterResponse> {
            unimplemented!()
        }
        async fn send_unregister_request(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: Option<String>,
        ) -> crate::transport::error::NetworkResult<actr_protocol::UnregisterResponse> {
            unimplemented!()
        }
        async fn send_heartbeat(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: actr_protocol::ServiceAvailabilityState,
            _: f32,
            _: f32,
        ) -> crate::transport::error::NetworkResult<actr_protocol::Pong> {
            unimplemented!()
        }
        async fn send_route_candidates_request(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: actr_protocol::RouteCandidatesRequest,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RouteCandidatesResponse>
        {
            unimplemented!()
        }
        async fn send_credential_update_request(
            &self,
            _: ActrId,
            _: AIdCredential,
        ) -> crate::transport::error::NetworkResult<actr_protocol::RegisterResponse> {
            unimplemented!()
        }
        async fn send_envelope(
            &self,
            _: actr_protocol::SignalingEnvelope,
        ) -> crate::transport::error::NetworkResult<()> {
            unimplemented!()
        }
        async fn receive_envelope(
            &self,
        ) -> crate::transport::error::NetworkResult<Option<actr_protocol::SignalingEnvelope>>
        {
            unimplemented!()
        }
        async fn get_signing_key(
            &self,
            _: ActrId,
            _: AIdCredential,
            _: u32,
        ) -> crate::transport::error::NetworkResult<(u32, Vec<u8>)> {
            Err(crate::transport::error::NetworkError::ConnectionError(
                "should not be called".into(),
            ))
        }
    }

    /// 构造带预置公钥的 WsAuthContext（key_id 已在缓存中，无需走 signaling）
    async fn make_auth_ctx(sk: &SigningKey, key_id: u32, local_actor: ActrId) -> WsAuthContext {
        let pubkey_bytes = sk.verifying_key().as_bytes().to_vec();
        let cache = AisKeyCache::new();
        cache.seed(key_id, &pubkey_bytes).await.unwrap();

        let local_credential = AIdCredential {
            key_id,
            claims: bytes::Bytes::new(),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        };
        let cred_state = crate::lifecycle::CredentialState::new(local_credential, None, None);

        WsAuthContext {
            ais_key_cache: cache,
            actor_id: local_actor,
            credential_state: cred_state,
            signaling_client: Arc::new(NullSignaling),
        }
    }

    // ─── verify_credential ────────────────────────────────────────────────────

    /// 正常路径：有效凭证 + actor_id 一致 → Some(())
    #[tokio::test]
    async fn verify_credential_valid_returns_some() {
        let sk = signing_key(1);
        let actor = test_actor_id(100);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, future_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        let result = WebSocketGate::verify_credential(&credential, &source_bytes, &ctx).await;
        assert!(result.is_some(), "valid credential should pass");
    }

    /// 签名被翻转 1 bit → 验签失败 → None
    #[tokio::test]
    async fn verify_credential_tampered_signature_returns_none() {
        let sk = signing_key(2);
        let actor = test_actor_id(101);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let mut credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        let mut sig = credential.signature.to_vec();
        sig[0] ^= 0xFF;
        credential.signature = sig.into();

        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);
        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// 签名字节数不足 64 字节 → try_into 失败 → None
    #[tokio::test]
    async fn verify_credential_short_signature_returns_none() {
        let sk = signing_key(3);
        let actor = test_actor_id(102);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let mut credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        credential.signature = bytes::Bytes::from(vec![0u8; 32]); // too short

        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);
        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// expires_at 在过去 → 已过期 → None
    #[tokio::test]
    async fn verify_credential_expired_returns_none() {
        let sk = signing_key(4);
        let actor = test_actor_id(103);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, past_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none(),
            "expired credential should be rejected"
        );
    }

    /// claims.actor_id 与 X-Actr-Source-ID 对应的 ActrId 不一致 → None（防身份欺骗）
    #[tokio::test]
    async fn verify_credential_actor_id_mismatch_returns_none() {
        let sk = signing_key(5);
        let claimed_actor = test_actor_id(200); // credential 声称是 200
        let actual_source = test_actor_id(201); // 实际连接方是 201
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;

        // credential 签名的是 claimed_actor(200)，但 source_id 是 actual_source(201)
        let credential = make_valid_credential(&sk, &claimed_actor, future_ts(), 1);
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actual_source);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none(),
            "actor_id mismatch should be rejected"
        );
    }

    /// IdentityClaims bytes 是非法 protobuf → 解码失败 → None
    #[tokio::test]
    async fn verify_credential_invalid_claims_proto_returns_none() {
        let sk = signing_key(6);
        let actor = test_actor_id(104);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;

        // 构造 claims 为垃圾字节，然后对其签名（签名本身有效，但 claims 无法解码）
        let garbage = b"\xFF\xFF\xFF\xFF\xFF";
        let signature = sk.sign(garbage);
        let credential = AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::from(garbage.to_vec()),
            signature: signature.to_bytes().to_vec().into(),
        };
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    /// source_id_bytes 是非法 protobuf → ActrId 解码失败 → None
    #[tokio::test]
    async fn verify_credential_invalid_source_id_returns_none() {
        let sk = signing_key(7);
        let actor = test_actor_id(105);
        let ctx = make_auth_ctx(&sk, 1, test_actor_id(999)).await;
        let credential = make_valid_credential(&sk, &actor, future_ts(), 1);

        let bad_source_id = b"\xFF\xFF\xFF\xFF"; // 无效 protobuf

        assert!(
            WebSocketGate::verify_credential(&credential, bad_source_id, &ctx)
                .await
                .is_none()
        );
    }

    /// key_id 不在缓存且 signaling 返回错误 → None（signaling fetch 失败）
    #[tokio::test]
    async fn verify_credential_unknown_key_id_returns_none() {
        let sk = signing_key(8);
        let actor = test_actor_id(106);
        // key_id=99 不在缓存，NullSignaling 会返回错误
        let cache = AisKeyCache::new();
        let local_credential = AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::new(),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        };
        let ctx = WsAuthContext {
            ais_key_cache: cache,
            actor_id: test_actor_id(999),
            credential_state: crate::lifecycle::CredentialState::new(local_credential, None, None),
            signaling_client: Arc::new(NullSignaling),
        };

        let credential = make_valid_credential(&sk, &actor, future_ts(), 99); // key_id=99 不存在
        let source_bytes = actr_protocol::prost::Message::encode_to_vec(&actor);

        assert!(
            WebSocketGate::verify_credential(&credential, &source_bytes, &ctx)
                .await
                .is_none()
        );
    }

    // ─── handle_envelope 路由逻辑 ─────────────────────────────────────────────

    struct CapturingMailbox {
        enqueue_count: AtomicUsize,
        last_priority: std::sync::Mutex<Option<MessagePriority>>,
    }

    impl CapturingMailbox {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                enqueue_count: AtomicUsize::new(0),
                last_priority: std::sync::Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl Mailbox for CapturingMailbox {
        async fn enqueue(
            &self,
            _from: Vec<u8>,
            _payload: Vec<u8>,
            priority: MessagePriority,
        ) -> StorageResult<Uuid> {
            self.enqueue_count.fetch_add(1, Ordering::SeqCst);
            *self.last_priority.lock().unwrap() = Some(priority);
            Ok(Uuid::new_v4())
        }
        async fn dequeue(&self) -> StorageResult<Vec<MessageRecord>> {
            Ok(vec![])
        }
        async fn ack(&self, _: Uuid) -> StorageResult<()> {
            Ok(())
        }
        async fn status(&self) -> StorageResult<MailboxStats> {
            Ok(MailboxStats {
                queued_messages: 0,
                inflight_messages: 0,
                queued_by_priority: Default::default(),
            })
        }
    }

    fn make_rpc_envelope(request_id: &str) -> RpcEnvelope {
        RpcEnvelope {
            request_id: request_id.to_string(),
            route_key: "test".to_string(),
            payload: Some(bytes::Bytes::from("hello")),
            error: None,
            timeout_ms: 5000,
            ..Default::default()
        }
    }

    fn empty_pending()
    -> Arc<RwLock<HashMap<String, (ActrId, oneshot::Sender<actr_protocol::ActorResult<Bytes>>)>>>
    {
        Arc::new(RwLock::new(HashMap::new()))
    }

    /// RPC Request（无 pending 条目）→ 入队 Mailbox，优先级 Normal
    #[tokio::test]
    async fn handle_envelope_request_goes_to_mailbox_with_normal_priority() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let envelope = make_rpc_envelope("req-1");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![1u8, 2, 3],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *mailbox.last_priority.lock().unwrap(),
            Some(MessagePriority::Normal)
        );
    }

    /// RpcSignal → 入队 Mailbox，优先级 High
    #[tokio::test]
    async fn handle_envelope_rpc_signal_uses_high_priority() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let envelope = make_rpc_envelope("sig-1");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcSignal,
            pending,
            mailbox.clone(),
        )
        .await;

        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            *mailbox.last_priority.lock().unwrap(),
            Some(MessagePriority::High)
        );
    }

    /// RPC Response（pending 中有对应 request_id）→ 唤醒等待方，不入 Mailbox
    #[tokio::test]
    async fn handle_envelope_response_resolves_pending_not_mailbox() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(1);

        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-2".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-2");
        envelope.payload = Some(bytes::Bytes::from("response-payload"));
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending.clone(),
            mailbox.clone(),
        )
        .await;

        assert_eq!(
            mailbox.enqueue_count.load(Ordering::SeqCst),
            0,
            "response must not go to mailbox"
        );
        let result = rx.await.expect("oneshot must be resolved");
        assert!(result.is_ok(), "response payload should resolve Ok");
    }

    /// Response 同时带 payload 和 error → Err(DecodeFailure) 发给等待方
    #[tokio::test]
    async fn handle_envelope_response_both_payload_and_error_gives_decode_failure() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(2);
        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-3".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-3");
        envelope.payload = Some(bytes::Bytes::from("x"));
        envelope.error = Some(actr_protocol::ErrorResponse {
            code: 500,
            message: "err".to_string(),
        });
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        let result = rx.await.unwrap();
        assert!(
            matches!(result, Err(crate::error::ActrError::DecodeFailure(_))),
            "both payload+error should produce DecodeFailure: {result:?}"
        );
    }

    /// Response 只含 error（无 payload）→ Err(Unavailable) 发给等待方
    #[tokio::test]
    async fn handle_envelope_response_error_only_gives_unavailable() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(3);
        let (tx, rx) = oneshot::channel();
        pending
            .write()
            .await
            .insert("req-4".to_string(), (actor, tx));

        let mut envelope = make_rpc_envelope("req-4");
        envelope.payload = None;
        envelope.error = Some(actr_protocol::ErrorResponse {
            code: 503,
            message: "unavailable".to_string(),
        });
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending,
            mailbox.clone(),
        )
        .await;

        let result = rx.await.unwrap();
        assert!(
            matches!(result, Err(crate::error::ActrError::Unavailable(_))),
            "error-only response should produce Unavailable: {result:?}"
        );
        assert_eq!(mailbox.enqueue_count.load(Ordering::SeqCst), 0);
    }

    /// Response 处理后，pending 中对应条目已被移除
    #[tokio::test]
    async fn handle_envelope_response_removes_pending_entry() {
        let mailbox = CapturingMailbox::new();
        let pending = empty_pending();
        let actor = test_actor_id(4);
        let (tx, _rx) = oneshot::channel::<actr_protocol::ActorResult<Bytes>>();
        pending
            .write()
            .await
            .insert("req-5".to_string(), (actor, tx));

        let envelope = make_rpc_envelope("req-5");
        let data = actr_protocol::prost::Message::encode_to_vec(&envelope);

        WebSocketGate::handle_envelope(
            envelope,
            vec![],
            bytes::Bytes::from(data),
            PayloadType::RpcReliable,
            pending.clone(),
            mailbox,
        )
        .await;

        assert!(
            !pending.read().await.contains_key("req-5"),
            "pending entry must be removed after response"
        );
    }
}
