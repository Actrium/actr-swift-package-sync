//! WebSocketServer - 入站 WebSocket 连接监听器
//!
//! 绑定 TCP 端口，接受对端节点主动发起的 WebSocket 连接（直连模式）。
//! 每个入站连接被封装为 `InboundWsConn`，通过 mpsc 通道传递给 `WebSocketGate`。
//!
//! ## 发送方身份识别与认证
//!
//! 连接节点须在 HTTP 升级请求中携带：
//! ```text
//! X-Actr-Source-ID:  <hex-encoded protobuf ActrId bytes>
//! X-Actr-Credential: <base64-encoded proto AIdCredential bytes>（可选）
//! ```
//! - `X-Actr-Source-ID` 缺失时 `source_id_bytes` 为空，响应路由将失败
//! - `X-Actr-Credential` 用于 Ed25519 签名验证（gate 层执行）；缺失时按配置决定是否拒绝连接

use super::connection::WebSocketConnection;
use crate::error::{ActorResult, ActrError};
use actr_protocol::AIdCredential;
use actr_protocol::prost::Message as ProstMessage;
use std::net::SocketAddr;
use std::sync::Mutex as StdMutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::MaybeTlsStream;
use tokio_util::sync::CancellationToken;

/// 入站 WebSocket 连接通知通道容量
const ACCEPT_CHANNEL_CAPACITY: usize = 64;

/// 入站连接信息：连接实例 + 发送方 ActrId bytes + 发送方 AIdCredential（可选）
pub type InboundWsConn = (WebSocketConnection, Vec<u8>, Option<AIdCredential>);

/// WebSocketServer - 监听入站 WebSocket 连接
///
/// 通道元素类型为 `InboundWsConn = (WebSocketConnection, Vec<u8>, Option<AIdCredential>)`。
///
/// # 使用方式
/// ```rust,ignore
/// let (server, mut rx) = WebSocketServer::bind(8090).await?;
/// server.start(shutdown_token);
///
/// while let Some((conn, source_id, credential)) = rx.recv().await {
///     gate.handle_inbound(conn, source_id, credential).await;
/// }
/// ```
pub struct WebSocketServer {
    listener: TcpListener,
    conn_tx: mpsc::Sender<InboundWsConn>,
    local_addr: SocketAddr,
}

impl WebSocketServer {
    /// 绑定到指定端口，返回 server 实例和入站连接接收端
    pub async fn bind(port: u16) -> ActorResult<(Self, mpsc::Receiver<InboundWsConn>)> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            ActrError::Internal(format!("WebSocketServer: failed to bind port {port}: {e}"))
        })?;
        let local_addr = listener.local_addr().map_err(|e| {
            ActrError::Internal(format!("WebSocketServer: failed to get local addr: {e}"))
        })?;

        let (conn_tx, conn_rx) = mpsc::channel(ACCEPT_CHANNEL_CAPACITY);

        tracing::info!("🔌 WebSocketServer bound on {}", local_addr);

        Ok((
            Self {
                listener,
                conn_tx,
                local_addr,
            },
            conn_rx,
        ))
    }

    /// 返回实际监听地址（端口为 0 时获取系统分配的端口）
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 启动 accept 循环（在后台 task 中运行）
    ///
    /// 每接受一个 TCP 连接就进行 WebSocket 升级，升级成功后封装为
    /// `InboundWsConn` 并送入通道，由 `WebSocketGate` 完成身份验证。
    ///
    /// 发送方身份通过以下 HTTP 头传递：
    /// - `X-Actr-Source-ID`：hex 编码的 ActrId protobuf bytes
    /// - `X-Actr-Credential`：base64 编码的 AIdCredential protobuf bytes（用于 Ed25519 验签）
    pub fn start(self, shutdown_token: CancellationToken) {
        tokio::spawn(async move {
            tracing::info!(
                "🚀 WebSocketServer accept loop started on {}",
                self.local_addr
            );

            loop {
                tokio::select! {
                    _ = shutdown_token.cancelled() => {
                        tracing::info!("🛑 WebSocketServer shutting down");
                        break;
                    }
                    accept_result = self.listener.accept() => {
                        match accept_result {
                            Ok((stream, peer_addr)) => {
                                tracing::debug!(
                                    "🔗 Incoming TCP connection from: {}",
                                    peer_addr
                                );

                                let conn_tx = self.conn_tx.clone();

                                // 在独立 task 中完成 WS 握手，避免阻塞 accept 循环
                                tokio::spawn(async move {
                                    // 使用 arc+std Mutex 在同步回调中捕获握手头信息
                                    let captured_source_id: std::sync::Arc<StdMutex<Vec<u8>>> =
                                        std::sync::Arc::new(StdMutex::new(Vec::new()));
                                    let captured_credential: std::sync::Arc<StdMutex<Option<AIdCredential>>> =
                                        std::sync::Arc::new(StdMutex::new(None));

                                    let capture_src = captured_source_id.clone();
                                    let capture_cred = captured_credential.clone();

                                    let callback = move |req: &tokio_tungstenite::tungstenite::handshake::server::Request,
                                                         res: tokio_tungstenite::tungstenite::handshake::server::Response|
                                     -> Result<
                                        tokio_tungstenite::tungstenite::handshake::server::Response,
                                        tokio_tungstenite::tungstenite::handshake::server::ErrorResponse,
                                    > {
                                        // 提取 X-Actr-Source-ID
                                        if let Some(val) = req.headers().get("X-Actr-Source-ID") {
                                            if let Ok(hex_str) = val.to_str() {
                                                match hex::decode(hex_str) {
                                                    Ok(bytes) => {
                                                        *capture_src.lock().unwrap() = bytes;
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "⚠️ Invalid X-Actr-Source-ID hex from {}: {}",
                                                            peer_addr,
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        } else {
                                            tracing::warn!(
                                                "⚠️ No X-Actr-Source-ID header from {} — response routing will fail",
                                                peer_addr
                                            );
                                        }

                                        // 提取 X-Actr-Credential（base64 → proto AIdCredential）
                                        if let Some(val) = req.headers().get("X-Actr-Credential") {
                                            if let Ok(b64_str) = val.to_str() {
                                                use base64::Engine as _;
                                                match base64::engine::general_purpose::STANDARD.decode(b64_str) {
                                                    Ok(cred_bytes) => {
                                                        match AIdCredential::decode(cred_bytes.as_slice()) {
                                                            Ok(credential) => {
                                                                *capture_cred.lock().unwrap() = Some(credential);
                                                            }
                                                            Err(e) => {
                                                                tracing::warn!(
                                                                    "⚠️ Invalid X-Actr-Credential proto from {}: {}",
                                                                    peer_addr, e
                                                                );
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "⚠️ Invalid X-Actr-Credential base64 from {}: {}",
                                                            peer_addr, e
                                                        );
                                                    }
                                                }
                                            }
                                        }

                                        Ok(res)
                                    };

                                    match tokio_tungstenite::accept_hdr_async(
                                        MaybeTlsStream::Plain(stream),
                                        callback,
                                    )
                                    .await
                                    {
                                        Ok(ws_stream) => {
                                            tracing::info!(
                                                "✅ WebSocket handshake completed from: {}",
                                                peer_addr
                                            );

                                            let source_id = captured_source_id.lock().unwrap().clone();
                                            let credential = captured_credential.lock().unwrap().take();

                                            let conn =
                                                WebSocketConnection::from_server_stream(ws_stream);

                                            if conn_tx.send((conn, source_id, credential)).await.is_err() {
                                                tracing::warn!(
                                                    "⚠️ WebSocketServer: conn_tx closed, dropping connection from {}",
                                                    peer_addr
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "❌ WebSocket handshake failed from {}: {}",
                                                peer_addr,
                                                e
                                            );
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::error!("❌ WebSocketServer accept error: {}", e);
                                // 短暂休眠后继续，避免高速错误循环
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }

            tracing::info!("🔌 WebSocketServer accept loop exited");
        });
    }
}
