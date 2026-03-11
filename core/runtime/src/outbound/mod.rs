//! Outbound Layer 2: Outbound gate abstraction layer
//!
//! 提供统一的出站消息发送接口，支持进程内和跨进程通信。
//!
//! # 设计特性
//!
//! - **Enum Dispatch**：使用枚举而非 trait object，实现零虚函数调用
//! - **零成本抽象**：编译时确定类型，静态分发
//! - **统一接口**：InprocOut 和 OutprocOut 共享相同的方法签名

mod inproc_out_gate;
mod outproc_out_gate;

pub use inproc_out_gate::InprocOutGate;
pub use outproc_out_gate::OutprocOutGate;

use actr_framework::{Bytes, MediaSample};
use actr_protocol::{ActorResult, ActrError, ActrId, PayloadType, RpcEnvelope};
use std::sync::Arc;

/// OutGate - 出站消息门枚举
///
/// # 设计原则
///
/// - 使用 **enum dispatch** 而非 trait object，避免虚函数调用
/// - **零成本抽象**：编译时准确确定类型
/// - **完全独立**：仅用于出站（Outbound），不包含任何入站路由逻辑
///
/// # 性能
///
/// ```text
/// OutGate::send_request() 内部：
///   match self {
///       OutGate::InprocOut(gate) => gate.send_request(...),   // ← 静态分发
///       OutGate::OutprocOut(gate) => gate.send_request(...),  // ← 静态分发
///   }
///
/// 性能：
///   - 无虚函数表查找
///   - 编译器完全内联
///   - CPU 分支预测命中率 >95%
/// ```
#[derive(Clone)]
pub enum OutGate {
    /// InprocOut - 进程内传输（零序列化，出站）
    InprocOut(Arc<InprocOutGate>),

    /// OutprocOut - 跨进程传输（Protobuf 序列化，出站）
    OutprocOut(Arc<OutprocOutGate>),
}

impl OutGate {
    /// 发送请求并等待响应
    ///
    /// # 参数
    ///
    /// - `target`: 目标 Actor ID
    /// - `envelope`: 消息信封（包含 route_key 和 payload）
    ///
    /// # 返回
    ///
    /// 返回响应的字节数据
    ///
    /// # 实现
    ///
    /// 使用 enum dispatch 静态分发到对应的实现：
    /// - `InprocOut`: 零序列化，直接传递 RpcEnvelope
    /// - `OutprocOut`: Protobuf 序列化，通过 Transport 层发送
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "OutGate.send_request")
    )]
    pub async fn send_request(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<Bytes> {
        match self {
            OutGate::InprocOut(gate) => gate.send_request(target, envelope).await,
            OutGate::OutprocOut(gate) => gate.send_request(target, envelope).await,
        }
    }

    /// 发送请求并等待响应（显式指定 PayloadType）
    pub async fn send_request_with_type(
        &self,
        target_id: &ActrId,
        payload_type: PayloadType,
        envelope: RpcEnvelope,
    ) -> ActorResult<Bytes> {
        match self {
            OutGate::InprocOut(gate) => {
                gate.send_request_with_type(target_id, payload_type, None, envelope)
                    .await
            }
            OutGate::OutprocOut(gate) => {
                gate.send_request_with_type(target_id, payload_type, envelope)
                    .await
            }
        }
    }

    /// 发送单向消息（不等待响应）
    ///
    /// # 参数
    ///
    /// - `target`: 目标 Actor ID
    /// - `envelope`: 消息信封
    ///
    /// # 语义
    ///
    /// Fire-and-forget：发送后立即返回，不等待响应
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "OutGate.send_message")
    )]
    pub async fn send_message(&self, target: &ActrId, envelope: RpcEnvelope) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(gate) => gate.send_message(target, envelope).await,
            OutGate::OutprocOut(gate) => gate.send_message(target, envelope).await,
        }
    }

    /// 发送单向消息（显式指定 PayloadType）
    pub async fn send_message_with_type(
        &self,
        target: &ActrId,
        payload_type: PayloadType,
        envelope: RpcEnvelope,
    ) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(gate) => {
                gate.send_message_with_type(target, payload_type, None, envelope)
                    .await
            }
            OutGate::OutprocOut(gate) => {
                gate.send_message_with_type(target, payload_type, envelope)
                    .await
            }
        }
    }

    /// 发送媒体样本（WebRTC native media）
    ///
    /// # 参数
    ///
    /// - `target`: 目标 Actor ID
    /// - `track_id`: Media track 标识符
    /// - `sample`: 媒体样本数据
    ///
    /// # 语义
    ///
    /// - 仅支持 OutprocOut（WebRTC）
    /// - InprocOut 返回 NotImplemented 错误
    /// - 使用 WebRTC RTCRtpSender 发送，无 protobuf 开销
    pub async fn send_media_sample(
        &self,
        target: &ActrId,
        track_id: &str,
        sample: MediaSample,
    ) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(_gate) => {
                // InprocOut does not support MediaTrack (WebRTC-specific feature)
                Err(ActrError::NotImplemented(
                    "MediaTrack is only supported for remote actors via WebRTC".to_string(),
                ))
            }
            OutGate::OutprocOut(gate) => gate.send_media_sample(target, track_id, sample).await,
        }
    }

    /// Add a media track to the WebRTC connection
    pub async fn add_media_track(
        &self,
        target: &ActrId,
        track_id: &str,
        codec: &str,
        media_type: &str,
    ) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(_gate) => Err(ActrError::NotImplemented(
                "MediaTrack is only supported for remote actors via WebRTC".to_string(),
            )),
            OutGate::OutprocOut(gate) => {
                gate.add_media_track(target, track_id, codec, media_type)
                    .await
            }
        }
    }

    /// Remove a media track from the WebRTC connection.
    pub async fn remove_media_track(&self, target: &ActrId, track_id: &str) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(_gate) => Err(ActrError::NotImplemented(
                "MediaTrack is only supported for remote actors via WebRTC".to_string(),
            )),
            OutGate::OutprocOut(gate) => gate.remove_media_track(target, track_id).await,
        }
    }

    /// 发送 DataStream（Fast Path 数据流）
    ///
    /// # 参数
    ///
    /// - `target`: 目标 Actor ID
    /// - `payload_type`: PayloadType (StreamReliable 或 StreamLatencyFirst)
    /// - `data`: 序列化后的 DataStream bytes
    ///
    /// # 语义
    ///
    /// - InprocOut: 通过 mpsc channel 发送
    /// - OutprocOut: 通过 WebRTC DataChannel 或 WebSocket 发送
    pub async fn send_data_stream(
        &self,
        target: &ActrId,
        payload_type: actr_protocol::PayloadType,
        data: Bytes,
    ) -> ActorResult<()> {
        match self {
            OutGate::InprocOut(gate) => gate.send_data_stream(target, payload_type, data).await,
            OutGate::OutprocOut(gate) => gate.send_data_stream(target, payload_type, data).await,
        }
    }
}
