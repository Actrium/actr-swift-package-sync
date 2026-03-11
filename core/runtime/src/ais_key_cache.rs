//! AisKeyCache - AIS signing 公钥本地缓存
//!
//! actor 注册时从 RegisterOk 获得当前 AIS signing 公钥，缓存于此。
//! 验签时按 key_id 查找；miss 时通过 signaling 拉取并写入缓存。
//! 公钥无需保密，缓存策略简单：按 key_id 永久保留（key_id 单调递增，条目极少）。

use crate::error::{ActorResult, ActrError};
use crate::wire::SignalingClient;
use actr_protocol::{AIdCredential, ActrId};
use ed25519_dalek::VerifyingKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// AIS Ed25519 signing 公钥缓存
///
/// 线程安全，通过 `Arc<AisKeyCache>` 共享使用。
/// 公钥按 key_id 永久存储；key_id 由 AIS 单调递增分配，实际条目数极少。
pub struct AisKeyCache {
    cache: RwLock<HashMap<u32, VerifyingKey>>,
}

impl AisKeyCache {
    /// 创建新的空缓存，返回 `Arc` 包装以便共享
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// 注册或续期时调用，将 AIS signing 公钥写入缓存
    ///
    /// `pubkey_bytes` 必须是 32 字节的 Ed25519 原始公钥。
    /// 若 key_id 已存在则覆盖（正常情况下不应发生，保持幂等）。
    pub async fn seed(&self, key_id: u32, pubkey_bytes: &[u8]) -> ActorResult<()> {
        let verifying_key = VerifyingKey::from_bytes(
            pubkey_bytes
                .try_into()
                .map_err(|_| ActrError::Internal("signing pubkey 必须为 32 字节".to_string()))?,
        )
        .map_err(|e| ActrError::Internal(format!("signing pubkey 无效: {e}")))?;

        self.cache.write().await.insert(key_id, verifying_key);
        tracing::debug!(key_id, "AisKeyCache: 写入公钥");
        Ok(())
    }

    /// 按 key_id 获取公钥；本地命中直接返回，miss 时通过 signaling 拉取并缓存
    ///
    /// 拉取失败视为不可恢复错误，由调用方决定是否重试。
    pub async fn get_or_fetch(
        &self,
        key_id: u32,
        actor_id: &ActrId,
        credential: &AIdCredential,
        signaling: &dyn SignalingClient,
    ) -> ActorResult<VerifyingKey> {
        // 先持读锁尝试命中，避免不必要的写锁竞争
        {
            let cache = self.cache.read().await;
            if let Some(key) = cache.get(&key_id) {
                tracing::trace!(key_id, "AisKeyCache: 命中缓存");
                return Ok(*key);
            }
        }

        // 缓存未命中，通过 signaling 拉取
        tracing::debug!(key_id, "AisKeyCache: 缓存未命中，向 signaling 拉取公钥");
        let (returned_key_id, pubkey_bytes) = signaling
            .get_signing_key(actor_id.clone(), credential.clone(), key_id)
            .await
            .map_err(|e| {
                tracing::warn!(key_id, error = ?e, "AisKeyCache: 拉取公钥失败");
                ActrError::Internal(format!("拉取 signing 公钥失败: {e:?}"))
            })?;

        let verifying_key =
            VerifyingKey::from_bytes(pubkey_bytes.as_slice().try_into().map_err(|_| {
                ActrError::Internal("拉取到的 signing pubkey 必须为 32 字节".to_string())
            })?)
            .map_err(|e| ActrError::Internal(format!("拉取到的 signing pubkey 无效: {e}")))?;

        self.cache
            .write()
            .await
            .insert(returned_key_id, verifying_key);
        tracing::debug!(
            key_id = returned_key_id,
            "AisKeyCache: 已缓存从 signaling 获取的公钥"
        );

        Ok(verifying_key)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::error::NetworkError;
    use actr_protocol::{ActrType, Realm};
    use async_trait::async_trait;
    use ed25519_dalek::SigningKey;

    fn test_actor_id() -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: 42,
            r#type: ActrType {
                manufacturer: "test".to_string(),
                name: "node".to_string(),
                version: "v1".to_string(),
            },
        }
    }

    fn dummy_credential() -> AIdCredential {
        AIdCredential {
            key_id: 1,
            claims: bytes::Bytes::new(),
            signature: bytes::Bytes::from(vec![0u8; 64]),
        }
    }

    /// 用固定种子生成可重复的 Ed25519 密钥对（无需 rand_core feature）
    fn test_signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn verifying_key_bytes(seed: u8) -> [u8; 32] {
        *test_signing_key(seed).verifying_key().as_bytes()
    }

    // ─── mock SignalingClient ──────────────────────────────────────────────

    struct MockSignaling {
        response: Option<(u32, Vec<u8>)>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl MockSignaling {
        fn ok(key_id: u32, bytes: Vec<u8>) -> Self {
            Self {
                response: Some((key_id, bytes)),
                calls: Default::default(),
            }
        }
        fn err() -> Self {
            Self {
                response: None,
                calls: Default::default(),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl crate::wire::SignalingClient for MockSignaling {
        async fn connect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        async fn disconnect(&self) -> crate::transport::error::NetworkResult<()> {
            Ok(())
        }
        fn is_connected(&self) -> bool {
            true
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
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &self.response {
                Some(r) => Ok(r.clone()),
                None => Err(NetworkError::ConnectionError("mock error".into())),
            }
        }
    }

    // ─── seed ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn seed_valid_32_bytes_succeeds() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &verifying_key_bytes(1)).await.is_ok());
    }

    #[tokio::test]
    async fn seed_31_bytes_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[0u8; 31]).await.is_err());
    }

    #[tokio::test]
    async fn seed_33_bytes_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[0u8; 33]).await.is_err());
    }

    #[tokio::test]
    async fn seed_empty_returns_error() {
        let cache = AisKeyCache::new();
        assert!(cache.seed(1, &[]).await.is_err());
    }

    #[tokio::test]
    async fn seed_twice_same_key_id_is_idempotent() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(2);
        cache.seed(1, &bytes).await.unwrap();
        cache.seed(1, &bytes).await.unwrap(); // must not error
        let mock = MockSignaling::err();
        let result = cache
            .get_or_fetch(1, &test_actor_id(), &dummy_credential(), &mock)
            .await;
        assert!(result.is_ok());
        assert_eq!(mock.calls(), 0, "seeded key must be hit from cache");
    }

    // ─── get_or_fetch: cache hit ─────────────────────────────────────────────

    #[tokio::test]
    async fn cache_hit_does_not_call_signaling() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(3);
        cache.seed(1, &bytes).await.unwrap();

        let mock = MockSignaling::err();
        let result = cache
            .get_or_fetch(1, &test_actor_id(), &dummy_credential(), &mock)
            .await;
        assert!(result.is_ok());
        assert_eq!(mock.calls(), 0);
    }

    #[tokio::test]
    async fn cache_hit_returns_correct_key() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(4);
        cache.seed(7, &bytes).await.unwrap();

        let mock = MockSignaling::err();
        let key = cache
            .get_or_fetch(7, &test_actor_id(), &dummy_credential(), &mock)
            .await
            .unwrap();
        assert_eq!(key.as_bytes(), &bytes);
    }

    // ─── get_or_fetch: cache miss ────────────────────────────────────────────

    #[tokio::test]
    async fn cache_miss_calls_signaling_and_caches_result() {
        let cache = AisKeyCache::new();
        let bytes = verifying_key_bytes(5);
        let mock = MockSignaling::ok(5, bytes.to_vec());

        let result = cache
            .get_or_fetch(5, &test_actor_id(), &dummy_credential(), &mock)
            .await;
        assert!(result.is_ok());
        assert_eq!(mock.calls(), 1);

        // second call must use cache
        let mock2 = MockSignaling::err();
        let result2 = cache
            .get_or_fetch(5, &test_actor_id(), &dummy_credential(), &mock2)
            .await;
        assert!(result2.is_ok());
        assert_eq!(mock2.calls(), 0, "second call should be cache hit");
    }

    #[tokio::test]
    async fn cache_miss_signaling_failure_returns_error() {
        let cache = AisKeyCache::new();
        let mock = MockSignaling::err();
        let result = cache
            .get_or_fetch(9, &test_actor_id(), &dummy_credential(), &mock)
            .await;
        assert!(result.is_err());
        assert_eq!(mock.calls(), 1);
    }

    #[tokio::test]
    async fn cache_miss_signaling_returns_31_byte_pubkey_returns_error() {
        let cache = AisKeyCache::new();
        let mock = MockSignaling::ok(3, vec![0u8; 31]);
        let result = cache
            .get_or_fetch(3, &test_actor_id(), &dummy_credential(), &mock)
            .await;
        assert!(result.is_err(), "invalid pubkey length should return error");
    }

    #[tokio::test]
    async fn different_key_ids_cached_independently() {
        let cache = AisKeyCache::new();
        cache.seed(1, &verifying_key_bytes(10)).await.unwrap();
        cache.seed(2, &verifying_key_bytes(20)).await.unwrap();

        let mock = MockSignaling::err();
        let k1 = cache
            .get_or_fetch(1, &test_actor_id(), &dummy_credential(), &mock)
            .await
            .unwrap();
        let k2 = cache
            .get_or_fetch(2, &test_actor_id(), &dummy_credential(), &mock)
            .await
            .unwrap();
        assert_ne!(k1.as_bytes(), k2.as_bytes());
        assert_eq!(mock.calls(), 0);
    }
}
