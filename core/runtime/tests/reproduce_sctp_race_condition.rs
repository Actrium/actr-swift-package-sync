//! 复现 SCTP 竞态条件问题
//!
//! 此测试在**未修复**的代码上运行，目标是复现：
//! 1. ICE restart 后 ICE 层报告 Connected
//! 2. 应用层立即发送 RPC 请求
//! 3. SCTP 层尚未就绪，导致写入失败
//! 4. 错误：No route to host (os error 65) 或 Connection closed
//!
//! 预期结果：
//! - 未修复代码：测试失败，检测到 SCTP 错误
//! - 修复后代码：测试通过，无 SCTP 错误

mod common;

use actr_protocol::{RpcEnvelope, prost::Message};
use actr_runtime::{
    ActrId,
    outbound::OutprocOutGate,
    transport::{
        DefaultWireBuilder, DefaultWireBuilderConfig, OutprocTransportManager,
        connection_event::{ConnectionEvent, ConnectionState},
    },
};
use common::{TestSignalingServer, create_peer_with_websocket, make_actor_id};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_reproduce_sctp_race_after_ice_restart() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .with_line_number(true)
        .with_file(true)
        .try_init()
        .ok();

    tracing::warn!("");
    tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    tracing::warn!("🧪 SCTP 竞态条件复现测试");
    tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    tracing::warn!("目标：在 ICE restart 后立即发送 RPC，触发 SCTP 错误");
    tracing::warn!("");

    // 启动信令服务器
    let server = TestSignalingServer::start().await.unwrap();

    let id_a = make_actor_id(100);
    let id_b = make_actor_id(200);

    // 创建两个 peer
    tracing::info!("📦 Creating peers...");
    let (coord_a, _client_a) = create_peer_with_websocket(id_a.clone(), &server.url())
        .await
        .unwrap();
    let (coord_b, _client_b) = create_peer_with_websocket(id_b.clone(), &server.url())
        .await
        .unwrap();

    // 创建 OutprocOutGate（用于发送 RPC）
    let wire_config_a = DefaultWireBuilderConfig::default();
    let wire_builder_a: Arc<dyn actr_runtime::transport::WireBuilder> = Arc::new(
        DefaultWireBuilder::new(Some(coord_a.clone()), wire_config_a),
    );
    let transport_mgr_a = Arc::new(OutprocTransportManager::new(id_a.clone(), wire_builder_a));
    let gate_a = Arc::new(OutprocOutGate::new(transport_mgr_a, Some(coord_a.clone())));

    // 为 peer B 创建 OutprocOutGate（用于响应）
    let wire_config_b = DefaultWireBuilderConfig::default();
    let wire_builder_b: Arc<dyn actr_runtime::transport::WireBuilder> = Arc::new(
        DefaultWireBuilder::new(Some(coord_b.clone()), wire_config_b),
    );
    let transport_mgr_b = Arc::new(OutprocTransportManager::new(id_b.clone(), wire_builder_b));
    let gate_b = Arc::new(OutprocOutGate::new(transport_mgr_b, Some(coord_b.clone())));

    // 启动 peer B 的 Echo 响应任务
    let responder_task = common::spawn_echo_responder(coord_b.clone(), gate_b.clone(), "Peer 200");

    // 启动 peer A 的响应接收任务
    let receiver_task =
        common::spawn_response_receiver(coord_a.clone(), gate_a.clone(), "Peer 100");

    // 建立初始连接
    tracing::info!("🔗 Establishing initial connection...");
    let ready_rx = coord_a.initiate_connection(&id_b).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), ready_rx)
        .await
        .expect("Connection timeout")
        .expect("Connection failed");

    tracing::info!("✅ Initial connection established");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 订阅连接事件
    let mut event_rx = coord_a.subscribe_events();

    // 触发 ICE restart
    tracing::warn!("");
    tracing::warn!("♻️ Triggering ICE restart (simulating network change)...");
    coord_a.restart_ice(&id_b).await.unwrap();

    // 监控连接状态并在 Connected 后立即发送
    let mut send_attempts = 0;
    let max_attempts = 3;
    let mut errors_detected = Vec::new();

    let timeout = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            Ok(event) = event_rx.recv() => {
                if let ConnectionEvent::StateChanged { peer_id, state, .. } = event {
                    if peer_id == id_b && state == ConnectionState::Connected {
                        send_attempts += 1;

                        tracing::warn!("");
                        tracing::warn!("🔥 Attempt {}/{}: ICE Connected detected!", send_attempts, max_attempts);
                        tracing::warn!("   Sending RPC requests immediately (without waiting)...");

                        // 立即发送多个 RPC 请求（增加触发概率）
                        for i in 0..10 {
                            let envelope = RpcEnvelope {
                                request_id: format!("test-{}-{}", send_attempts, i),
                                route_key: "test.ping".to_string(),
                                payload: Some(bytes::Bytes::from(format!("attempt {} msg {}", send_attempts, i))),
                                timeout_ms: 5000,
                                ..Default::default()
                            };

                            // 使用 OutboundGate 发送（公共 API）
                            let result = gate_a.send_request(&id_b, envelope).await;

                            match result {
                                Ok(_) => {
                                    tracing::debug!("   ✓ Message {} sent successfully", i);
                                }
                                Err(e) => {
                                    let err_str = e.to_string();
                                    tracing::error!("   ✗ Message {} failed: {}", i, err_str);

                                    // 检查是否是 SCTP 相关错误
                                    if err_str.contains("No route to host")
                                        || err_str.contains("os error 65")
                                        || err_str.contains("Connection closed")
                                        || err_str.contains("Transport error")
                                        || err_str.contains("EHOSTUNREACH") {
                                        errors_detected.push((send_attempts, i, err_str.clone()));
                                        tracing::error!("   🎯 SCTP-related error detected!");
                                    }
                                }
                            }

                            // 极短延迟（模拟快速发送）
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }

                        if send_attempts >= max_attempts {
                            break;
                        }
                    }
                }
            }
            _ = &mut timeout => {
                tracing::warn!("⏱️ Timeout reached");
                break;
            }
        }
    }

    // 分析结果
    tracing::warn!("");
    tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    tracing::warn!("📊 测试结果分析");
    tracing::warn!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    tracing::warn!("发送尝试次数: {}", send_attempts);
    tracing::warn!("检测到的错误: {}", errors_detected.len());

    if !errors_detected.is_empty() {
        tracing::error!("");
        tracing::error!("✅ 成功复现 SCTP 竞态条件！");
        tracing::error!("");
        tracing::error!("错误详情：");
        for (attempt, msg_id, error) in &errors_detected {
            tracing::error!("  - Attempt {}, Message {}: {}", attempt, msg_id, error);
        }
        tracing::error!("");
        tracing::error!("🔍 问题分析：");
        tracing::error!("  ICE 层报告 Connected，但 SCTP 层尚未就绪");
        tracing::error!("  应用层立即发送数据导致写入失败");
        tracing::error!("");
        tracing::error!("💡 解决方案：");
        tracing::error!("  在 ICE Connected 后等待 DataChannel Open");
        tracing::error!("  确保 SCTP 层完全就绪后再标记连接可用");
        tracing::error!("");

        panic!(
            "SCTP race condition reproduced: {} errors detected",
            errors_detected.len()
        );
    } else {
        tracing::warn!("");
        tracing::warn!("⚠️ 未能复现 SCTP 竞态条件");
        tracing::warn!("");
        tracing::warn!("可能原因：");
        tracing::warn!("  1. 本地网络太快（localhost），竞态窗口太小");
        tracing::warn!("  2. 时机不够精确（需要更多尝试）");
        tracing::warn!("  3. 修复已经生效（如果在修复后的代码上运行）");
        tracing::warn!("");
        tracing::warn!("建议：");
        tracing::warn!("  - 运行多次测试（10-20 次）");
        tracing::warn!("  - 在真实网络环境测试");
        tracing::warn!("  - 检查是否在未修复的代码上运行");
        tracing::warn!("");

        // 注意：即使未复现，测试也通过（因为这是概率性问题）
        // 但会在日志中明确说明
    }

    // 清理任务
    responder_task.abort();
    receiver_task.abort();
}
