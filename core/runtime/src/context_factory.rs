//! Context factory
//!
//! 负责创建 RuntimeContext 实例，注入 OutGate 和其他依赖。

use crate::context::RuntimeContext;
use crate::inbound::{DataStreamRegistry, MediaFrameRegistry};
use crate::outbound::OutGate;
use crate::transport::InprocTransportManager;
use crate::wire::webrtc::SignalingClient;
use actr_config::lock::LockFile;
use actr_protocol::{AIdCredential, ActrId};
use std::sync::Arc;

/// Context factory
///
/// # 职责
///
/// - 创建 RuntimeContext 实例
/// - 注入 OutGate（enum dispatch，零虚函数）
/// - 管理默认配置
#[derive(Clone)]
pub struct ContextFactory {
    /// 进程内通信 gate（本地调用）- 立即可用
    pub(crate) inproc_gate: OutGate,

    /// 跨进程通信 gate（远程调用）- 延迟初始化
    pub(crate) outproc_gate: Option<OutGate>,

    /// Shell → Workload 方向的传输管理器
    pub(crate) shell_to_workload: Arc<InprocTransportManager>,

    /// Workload → Shell 方向的传输管理器
    pub(crate) workload_to_shell: Arc<InprocTransportManager>,

    /// DataStream 回调注册表
    pub(crate) data_stream_registry: Arc<DataStreamRegistry>,

    /// MediaTrack 回调注册表
    pub(crate) media_frame_registry: Arc<MediaFrameRegistry>,

    /// Signaling client for discovery
    pub(crate) signaling_client: Arc<dyn SignalingClient>,

    /// Actr.lock.toml for dependency fingerprint lookups
    pub(crate) actr_lock: Option<LockFile>,
}

impl ContextFactory {
    /// 创建新的 ContextFactory
    ///
    /// # 参数
    ///
    /// - `inproc_gate`: 进程内通信 gate（Dest::Shell/Local）- 立即可用
    /// - `shell_to_workload`: Shell → Workload 方向的传输管理器
    /// - `workload_to_shell`: Workload → Shell 方向的传输管理器
    /// - `data_stream_registry`: DataStream 回调注册表
    /// - `media_frame_registry`: MediaTrack 回调注册表
    ///
    /// # 设计说明
    ///
    /// - **inproc_gate**: 在 ActrSystem::new() 时创建，立即可用（Shell/Local 通信不需要 ActorId）
    /// - **outproc_gate**: 初始为 None，在 ActrNode::start() WebRTC 初始化完成后设置
    /// - **双向 InprocTransportManager**: 确保 Shell 和 Workload 的 pending_requests 完全分离
    /// - **data_stream_registry**: 管理 DataStream 回调，支持应用数据流传输
    /// - **media_frame_registry**: 管理 MediaTrack 回调，支持 WebRTC 原生媒体流
    pub fn new(
        inproc_gate: OutGate,
        shell_to_workload: Arc<InprocTransportManager>,
        workload_to_shell: Arc<InprocTransportManager>,
        data_stream_registry: Arc<DataStreamRegistry>,
        media_frame_registry: Arc<MediaFrameRegistry>,
        signaling_client: Arc<dyn SignalingClient>,
    ) -> Self {
        Self {
            inproc_gate,
            outproc_gate: None, // 延迟初始化，等待 WebRTC 就绪
            shell_to_workload,
            workload_to_shell,
            data_stream_registry,
            media_frame_registry,
            signaling_client,
            actr_lock: None, // 延迟设置，在 ActrNode::start() 时从 actr_node.actr_lock 传入
        }
    }

    /// 设置跨进程通信 gate
    ///
    /// # 用途
    ///
    /// ActrNode::start() 完成 WebRTC 初始化后调用
    pub fn set_outproc_gate(&mut self, gate: OutGate) {
        tracing::debug!("🔄 Setting outproc OutGate in ContextFactory");
        self.outproc_gate = Some(gate);
    }

    /// 设置 Actr.lock.toml 文件
    ///
    /// # 用途
    ///
    /// ActrNode::start() 时调用，用于 discover_route_candidate 获取依赖 fingerprint
    pub fn set_actr_lock(&mut self, actr_lock: LockFile) {
        tracing::debug!("🔄 Setting actr_lock in ContextFactory");
        self.actr_lock = Some(actr_lock);
    }

    /// 获取 Shell → Workload 方向的传输管理器
    pub fn shell_to_workload(&self) -> Arc<InprocTransportManager> {
        self.shell_to_workload.clone()
    }

    /// 获取 Workload → Shell 方向的传输管理器
    pub fn workload_to_shell(&self) -> Arc<InprocTransportManager> {
        self.workload_to_shell.clone()
    }

    /// 创建 Context（用于消息处理）
    ///
    /// # 参数
    ///
    /// - `self_id`: 当前 Actor ID
    /// - `caller_id`: 调用方 Actor ID（可选）
    /// - `request_id`: 请求唯一 ID
    ///
    /// # 返回
    ///
    /// 返回 RuntimeContext 实例（实现了 Context trait）
    pub fn create(
        &self,
        self_id: &ActrId,
        caller_id: Option<&ActrId>,
        request_id: &str,
        credential: &AIdCredential,
    ) -> RuntimeContext {
        RuntimeContext::new(
            self_id.clone(),
            caller_id.cloned(),
            request_id.to_string(),
            self.inproc_gate.clone(), // Clone OutGate enum（Arc 内部，开销低）
            self.outproc_gate.clone(), // Clone Option<OutGate>
            self.data_stream_registry.clone(), // Clone Arc<DataStreamRegistry>
            self.media_frame_registry.clone(), // Clone Arc<MediaFrameRegistry>
            self.signaling_client.clone(),
            credential.clone(),
            self.actr_lock.clone(), // Clone Option<LockFile>
        )
    }

    /// 创建引导 Context（用于生命周期钩子）
    ///
    /// # 用途
    ///
    /// 用于 on_start/on_stop 钩子，无 caller_id
    pub fn create_bootstrap(&self, self_id: &ActrId, credential: &AIdCredential) -> RuntimeContext {
        RuntimeContext::new(
            self_id.clone(),
            None,
            uuid::Uuid::new_v4().to_string(),
            self.inproc_gate.clone(),
            self.outproc_gate.clone(),
            self.data_stream_registry.clone(),
            self.media_frame_registry.clone(),
            self.signaling_client.clone(),
            credential.clone(),
            self.actr_lock.clone(),
        )
    }
}
