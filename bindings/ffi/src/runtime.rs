//! Runtime wrappers for UniFFI export

use crate::error::{ActrError, ActrResult};
use crate::types::{ActrId, ActrType, NetworkEventResult};
use crate::workload::{DynamicWorkload, WorkloadBridge};
use actr_config::Config;
use actr_protocol::{ActrIdExt, ActrTypeExt};
use actr_runtime::{ActrNode, ActrRef, ActrSystem, NetworkEventHandle};
use bytes::Bytes;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Wrapper for ActrSystem - the entry point for creating actors
#[derive(uniffi::Object)]
pub struct ActrSystemWrapper {
    inner: Mutex<Option<ActrSystem>>,
    config: Config,
    network_event_handle: Mutex<Option<NetworkEventHandle>>,
}

#[uniffi::export]
impl ActrSystemWrapper {
    /// Create a new ActrSystem from configuration file
    #[uniffi::constructor(async_runtime = "tokio")]
    pub async fn new_from_file(config_path: String) -> ActrResult<Arc<Self>> {
        // Parse configuration first to get observability settings
        let config = actr_config::ConfigParser::from_file(&config_path).map_err(|e| {
            ActrError::ConfigError {
                msg: format!("Failed to parse config file at {}: {}", config_path, e),
            }
        })?;

        // Initialize logger based on configuration
        crate::logger::init_observability(config.observability.clone());

        info!(
            "Creating ActrSystem with signaling_url={}, realm_id={}",
            config.signaling_url, config.realm.realm_id
        );

        info!("Connecting to signaling server...");
        let system = ActrSystem::new(config.clone()).await.map_err(|e| {
            error!("Failed to create ActrSystem: {}", e);
            ActrError::InternalError {
                msg: format!("Failed to create ActrSystem: {e}"),
            }
        })?;

        info!("ActrSystem created successfully");

        Ok(Arc::new(Self {
            inner: Mutex::new(Some(system)),
            config,
            network_event_handle: Mutex::new(None),
        }))
    }

    /// Attach a workload and create an ActrNode
    pub fn attach(
        self: Arc<Self>,
        callback: Box<dyn WorkloadBridge>,
    ) -> ActrResult<Arc<ActrNodeWrapper>> {
        let system = self
            .inner
            .lock()
            .take()
            .ok_or_else(|| ActrError::StateError {
                msg: "ActrSystem already consumed".to_string(),
            })?;

        let workload = DynamicWorkload::new(Arc::from(callback));
        let node = system.attach(workload);

        Ok(Arc::new(ActrNodeWrapper {
            inner: Mutex::new(Some(node)),
            config: self.config.clone(),
        }))
    }

    /// Create a network event handle for platform callbacks.
    ///
    /// This must be called before `attach()` or after a previous handle was created.
    pub fn create_network_event_handle(&self) -> ActrResult<Arc<NetworkEventHandleWrapper>> {
        let mut handle_guard = self.network_event_handle.lock();
        if let Some(handle) = handle_guard.as_ref() {
            return Ok(Arc::new(NetworkEventHandleWrapper {
                inner: handle.clone(),
            }));
        }

        let system_guard = self.inner.lock();
        let system = system_guard.as_ref().ok_or_else(|| ActrError::StateError {
            msg: "ActrSystem already consumed".to_string(),
        })?;

        // Use default debounce behavior (0 = default).
        let handle = system.create_network_event_handle(0);
        *handle_guard = Some(handle.clone());

        Ok(Arc::new(NetworkEventHandleWrapper { inner: handle }))
    }
}

/// Wrapper for NetworkEventHandle - network lifecycle callbacks
#[derive(uniffi::Object)]
pub struct NetworkEventHandleWrapper {
    inner: NetworkEventHandle,
}

#[uniffi::export(async_runtime = "tokio")]
impl NetworkEventHandleWrapper {
    /// Handle network available event
    pub async fn handle_network_available(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_available()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network lost event
    pub async fn handle_network_lost(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_lost()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Handle network type changed event
    pub async fn handle_network_type_changed(
        &self,
        is_wifi: bool,
        is_cellular: bool,
    ) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .handle_network_type_changed(is_wifi, is_cellular)
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }

    /// Cleanup all connections (does not depend on network events).
    pub async fn cleanup_connections(&self) -> ActrResult<NetworkEventResult> {
        let result = self
            .inner
            .cleanup_connections()
            .await
            .map_err(|e| ActrError::InternalError { msg: e })?;
        Ok(result.into())
    }
}

/// Wrapper for ActrNode - a node ready to start
#[derive(uniffi::Object)]
pub struct ActrNodeWrapper {
    inner: Mutex<Option<ActrNode<DynamicWorkload>>>,
    #[allow(dead_code)]
    config: Config,
}

#[uniffi::export(async_runtime = "tokio")]
impl ActrNodeWrapper {
    /// Start the actor node and return an ActrRef
    pub async fn start(self: Arc<Self>) -> ActrResult<Arc<ActrRefWrapper>> {
        let node = self
            .inner
            .lock()
            .take()
            .ok_or_else(|| ActrError::StateError {
                msg: "ActrNode already started".to_string(),
            })?;

        let actr_ref = node.start().await.map_err(|e| ActrError::ConnectionError {
            msg: format!("Failed to start actor: {e}"),
        })?;

        Ok(Arc::new(ActrRefWrapper { inner: actr_ref }))
    }
}

/// Wrapper for ActrRef - a reference to a running actor
#[derive(uniffi::Object)]
pub struct ActrRefWrapper {
    inner: ActrRef<DynamicWorkload>,
}

#[uniffi::export(async_runtime = "tokio")]
impl ActrRefWrapper {
    /// Get the actor's ID
    pub fn actor_id(&self) -> ActrId {
        self.inner.actor_id().clone().into()
    }

    /// Discover actors of the specified type
    pub async fn discover(&self, target_type: ActrType, count: u32) -> ActrResult<Vec<ActrId>> {
        let proto_type: actr_protocol::ActrType = target_type.into();
        info!(
            "discover: looking for {} (count={count})",
            proto_type.to_string_repr(),
        );

        match self
            .inner
            .discover_route_candidates(&proto_type, count)
            .await
        {
            Ok(ids) => {
                info!("discover: found {} candidates", ids.len());
                for id in &ids {
                    debug!("  - candidate: {}", id.to_string_repr());
                }
                Ok(ids.into_iter().map(|id| id.into()).collect())
            }
            Err(e) => {
                error!("discover failed: {}", e);
                Err(ActrError::RpcError {
                    msg: format!("Discovery failed: {e}"),
                })
            }
        }
    }

    /// Trigger shutdown
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    /// Wait for shutdown to complete
    pub async fn wait_for_shutdown(&self) {
        self.inner.wait_for_shutdown().await;
    }

    /// Check if the actor is shutting down
    pub fn is_shutting_down(&self) -> bool {
        self.inner.is_shutting_down()
    }

    /// Call a remote actor via RPC proxy
    ///
    /// This sends a request through the local workload's RPC proxy mechanism,
    /// which forwards the call to the remote actor via WebRTC.
    ///
    /// # Arguments
    /// - `route_key`: RPC route key (e.g., "echo.EchoService/Echo")
    /// - `payload_type`: Payload transmission type (RpcReliable, RpcSignal, etc.)
    /// - `request_payload`: Request payload bytes (protobuf encoded)
    /// - `timeout_ms`: Timeout in milliseconds
    ///
    /// # Returns
    /// Response payload bytes (protobuf encoded)
    pub async fn call(
        &self,
        route_key: String,
        payload_type: crate::types::PayloadType,
        request_payload: Vec<u8>,
        timeout_ms: i64,
    ) -> ActrResult<Vec<u8>> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        info!("call_remote route: {route_key}");

        // Send request and wait for response
        let response_bytes = self
            .inner
            .call_raw(
                route_key,
                Bytes::from(request_payload),
                timeout_ms,
                proto_payload_type,
            )
            .await?;

        Ok(response_bytes.to_vec())
    }

    /// Send a one-way message to an actor (fire-and-forget)
    ///
    /// # Arguments
    /// - `route_key`: RPC route key (e.g., "echo.EchoService/Echo")
    /// - `payload_type`: Payload transmission type (RpcReliable, RpcSignal, etc.)
    /// - `message_payload`: Message payload bytes (protobuf encoded)
    pub async fn tell(
        &self,
        route_key: String,
        payload_type: crate::types::PayloadType,
        message_payload: Vec<u8>,
    ) -> ActrResult<()> {
        let proto_payload_type: actr_protocol::PayloadType = payload_type.into();
        info!("tell route: {route_key}");

        // Send message without waiting for response
        self.inner
            .tell_raw(route_key, Bytes::from(message_payload), proto_payload_type)
            .await?;

        Ok(())
    }
}
