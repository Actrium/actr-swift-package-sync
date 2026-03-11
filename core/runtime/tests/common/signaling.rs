//! WebSocket-based test signaling server
//!
//! Provides a real WebSocket signaling server for integration tests

use actr_protocol::{AIdCredential, SignalingEnvelope};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, RwLock, mpsc};
use tokio::time::sleep;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// Controllable test signaling server with real WebSocket
pub struct TestSignalingServer {
    port: u16,
    is_running: Arc<AtomicBool>,
    message_count: Arc<AtomicU32>,
    ice_restart_offer_count: Arc<AtomicU32>,
    /// Control: when true, server will accept connections but not forward messages
    pause_forwarding: Arc<AtomicBool>,
    connection_count: Arc<AtomicU32>,
    disconnection_count: Arc<AtomicU32>,
    #[allow(dead_code)]
    received_messages: Arc<Mutex<Vec<SignalingEnvelope>>>,
    /// Cancellation token: shutdown() 和 Drop 通过它关闭 accept loop 及所有 handler task
    cancel: tokio_util::sync::CancellationToken,
}

impl TestSignalingServer {
    /// Start the test server on a random available port
    pub async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        Self::start_with_listener(listener).await
    }

    /// Start the test server on the specified port (for restart scenarios where the URL must stay fixed).
    pub async fn start_on_port(port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await?;
        Self::start_with_listener(listener).await
    }

    async fn start_with_listener(listener: TcpListener) -> anyhow::Result<Self> {
        let port = listener.local_addr()?.port();
        let is_running = Arc::new(AtomicBool::new(true));
        let message_count = Arc::new(AtomicU32::new(0));
        let ice_restart_offer_count = Arc::new(AtomicU32::new(0));
        let connection_count = Arc::new(AtomicU32::new(0));
        let disconnection_count = Arc::new(AtomicU32::new(0));
        let received_messages = Arc::new(Mutex::new(Vec::new()));
        let pause_forwarding = Arc::new(AtomicBool::new(false));
        let cancel = tokio_util::sync::CancellationToken::new();

        // Clone for task
        let is_running_clone = is_running.clone();
        let message_count_clone = message_count.clone();
        let ice_restart_offer_count_clone = ice_restart_offer_count.clone();
        let received_messages_clone = received_messages.clone();
        let pause_forwarding_clone = pause_forwarding.clone();
        let connection_count_clone = connection_count.clone();
        let disconnection_count_clone = disconnection_count.clone();
        let cancel_clone = cancel.clone();

        // Spawn server task
        tokio::spawn(async move {
            Self::run_server(
                listener,
                is_running_clone,
                message_count_clone,
                ice_restart_offer_count_clone,
                received_messages_clone,
                pause_forwarding_clone,
                connection_count_clone,
                disconnection_count_clone,
                cancel_clone,
            )
            .await
        });

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        Ok(Self {
            port,
            is_running,
            message_count,
            ice_restart_offer_count,
            received_messages,
            pause_forwarding,
            connection_count,
            disconnection_count,
            cancel,
        })
    }

    async fn run_server(
        listener: TcpListener,
        is_running: Arc<AtomicBool>,
        message_count: Arc<AtomicU32>,
        ice_restart_offer_count: Arc<AtomicU32>,
        received_messages: Arc<Mutex<Vec<SignalingEnvelope>>>,
        pause_forwarding: Arc<AtomicBool>,
        connection_count: Arc<AtomicU32>,
        disconnection_count: Arc<AtomicU32>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        let clients: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("🛑 Test server shutting down");
                    is_running.store(false, Ordering::Release);
                    break;
                }

                accept_result = listener.accept() => {
                    if let Ok((stream, addr)) = accept_result {
                        tracing::info!("📥 New connection from {}", addr);
                        connection_count.fetch_add(1, Ordering::SeqCst);

                        let clients_clone = clients.clone();
                        let message_count_clone = message_count.clone();
                        let ice_restart_offer_count_clone = ice_restart_offer_count.clone();
                        let received_messages_clone = received_messages.clone();
                        let pause_forwarding_clone = pause_forwarding.clone();
                        let disconnection_count_clone = disconnection_count.clone();
                        let cancel_clone = cancel.clone();

                        tokio::spawn(async move {
                            if let Ok(ws_stream) = accept_async(stream).await {
                                let (mut ws_tx, mut ws_rx) = ws_stream.split();
                                let (client_tx, mut client_rx) = mpsc::unbounded_channel();
                                let client_id = uuid::Uuid::new_v4().to_string();

                                // Register client
                                clients_clone.write().await.insert(client_id.clone(), client_tx);

                                // Handle messages
                                loop {
                                    tokio::select! {
                                        // Shutdown signal: close WebSocket gracefully so client gets EOF
                                        _ = cancel_clone.cancelled() => {
                                            tracing::info!("🔌 Handler closing due to server shutdown");
                                            let _ = ws_tx.send(Message::Close(None)).await;
                                            break;
                                        }

                                        // Receive from client
                                        msg = ws_rx.next() => {
                                            match msg {
                                                Some(Ok(Message::Binary(data))) => {
                                                    message_count_clone.fetch_add(1, Ordering::Relaxed);

                                                    if let Ok(envelope) = actr_protocol::prost::Message::decode(&data[..]) {
                                                        received_messages_clone.lock().await.push(envelope);

                                                        // Decode again for processing
                                                        if let Ok(envelope) = actr_protocol::prost::Message::decode(&data[..]) {
                                                            Self::process_envelope(
                                                                envelope,
                                                                &client_id,
                                                                &clients_clone,
                                                                &ice_restart_offer_count_clone,
                                                                &pause_forwarding_clone,
                                                            ).await;
                                                        }
                                                    }
                                                }
                                                Some(Ok(Message::Close(_))) | None => {
                                                    break;
                                                }
                                                _ => {}
                                            }
                                        }

                                        // Send to client
                                        msg = client_rx.recv() => {
                                            if let Some(msg) = msg {                                                if ws_tx.send(msg).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                }

                                // Unregister client
                                clients_clone.write().await.remove(&client_id);
                                disconnection_count_clone.fetch_add(1, Ordering::SeqCst);
                            }
                        });
                    }
                }
            }
        }
    }

    async fn process_envelope(
        envelope: SignalingEnvelope,
        sender_id: &str,
        clients: &Arc<RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>>,
        ice_restart_offer_count: &Arc<AtomicU32>,
        pause_forwarding: &Arc<AtomicBool>,
    ) {
        if let Some(actr_protocol::signaling_envelope::Flow::ActrRelay(relay)) =
            envelope.flow.as_ref()
        {
            // Track ICE restart offers (COUNT BEFORE PAUSE CHECK)
            if let Some(actr_protocol::actr_relay::Payload::SessionDescription(sd)) =
                relay.payload.as_ref()
            {
                if sd.r#type == 3 {
                    // IceRestartOffer
                    ice_restart_offer_count.fetch_add(1, Ordering::SeqCst);
                    tracing::info!(
                        "📊 ICE restart offer detected (total: {})",
                        ice_restart_offer_count.load(Ordering::SeqCst)
                    );
                }
            }

            // NOW check pause for forwarding behaviors
            if pause_forwarding.load(Ordering::Acquire) {
                // Return early - do not reply to RoleNegotiation and do not Forward
                return;
            }

            // Handle RoleNegotiation: server decides roles and notifies BOTH parties
            // (matching real signaling server behavior in actrix/crates/signaling/src/server.rs)
            if let Some(actr_protocol::actr_relay::Payload::RoleNegotiation(role_neg)) =
                relay.payload.as_ref()
            {
                // RoleAssignment semantics:
                // - `source`: the REMOTE peer (from the receiver's perspective)
                // - `target`: the LOCAL peer (the receiver itself)
                // - `is_offerer`: whether the RECEIVER should act as offerer
                //
                // is_offerer is determined by serial_number comparison:
                //   from.serial > to.serial → from is offerer
                let from_is_offerer = role_neg.from.serial_number < role_neg.to.serial_number;

                // Build RoleAssignment for `from` (the requester)
                // source = to (remote), target = from (self), is_offerer = from_is_offerer
                let envelope_for_from = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: None,
                    timestamp: prost_types::Timestamp {
                        seconds: chrono::Utc::now().timestamp(),
                        nanos: 0,
                    },
                    flow: Some(actr_protocol::signaling_envelope::Flow::ActrRelay(
                        actr_protocol::ActrRelay {
                            source: role_neg.to.clone(), // remote peer for `from`
                            credential: AIdCredential::default(),
                            target: role_neg.from.clone(), // `from` itself
                            payload: Some(actr_protocol::actr_relay::Payload::RoleAssignment(
                                actr_protocol::RoleAssignment {
                                    is_offerer: from_is_offerer,
                                    remote_fixed: None,
                                },
                            )),
                        },
                    )),
                    traceparent: None,
                    tracestate: None,
                };

                // Build RoleAssignment for `to` (the other party)
                // source = from (remote), target = to (self), is_offerer = !from_is_offerer
                let envelope_for_to = SignalingEnvelope {
                    envelope_version: 1,
                    envelope_id: uuid::Uuid::new_v4().to_string(),
                    reply_for: None,
                    timestamp: prost_types::Timestamp {
                        seconds: chrono::Utc::now().timestamp(),
                        nanos: 0,
                    },
                    flow: Some(actr_protocol::signaling_envelope::Flow::ActrRelay(
                        actr_protocol::ActrRelay {
                            source: role_neg.from.clone(), // remote peer for `to`
                            credential: AIdCredential::default(),
                            target: role_neg.to.clone(), // `to` itself
                            payload: Some(actr_protocol::actr_relay::Payload::RoleAssignment(
                                actr_protocol::RoleAssignment {
                                    is_offerer: !from_is_offerer,
                                    remote_fixed: None,
                                },
                            )),
                        },
                    )),
                    traceparent: None,
                    tracestate: None,
                };

                // Send to both parties
                let clients_read = clients.read().await;
                for (id, tx) in clients_read.iter() {
                    if id == sender_id {
                        // Send to `from` (the requester)
                        let encoded =
                            actr_protocol::prost::Message::encode_to_vec(&envelope_for_from);
                        let _ = tx.send(Message::Binary(encoded.into()));
                    } else {
                        // Send to `to` (the other party)
                        let encoded =
                            actr_protocol::prost::Message::encode_to_vec(&envelope_for_to);
                        let _ = tx.send(Message::Binary(encoded.into()));
                    }
                }
                return;
            }

            // Forward to all other clients (broadcast)
            let clients_read = clients.read().await;
            let encoded = actr_protocol::prost::Message::encode_to_vec(&envelope);
            for (id, tx) in clients_read.iter() {
                if id != sender_id {
                    let _ = tx.send(Message::Binary(encoded.clone().into()));
                }
            }
        }
    }

    /// Get server URL
    pub fn url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }

    /// Get server port
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Shutdown the server (simulates total signaling unavailability)
    pub async fn shutdown(&mut self) {
        if !self.cancel.is_cancelled() {
            tracing::warn!("🔴 Shutting down test signaling server");
            self.cancel.cancel();
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// Pause message forwarding (simulates signaling connected but not working)
    pub fn pause_forwarding(&self) {
        tracing::warn!("⏸️  Pausing message forwarding");
        self.pause_forwarding.store(true, Ordering::Release);
    }

    /// Resume message forwarding
    pub fn resume_forwarding(&self) {
        tracing::info!("▶️  Resuming message forwarding");
        self.pause_forwarding.store(false, Ordering::Release);
    }

    /// Get message count
    pub fn message_count(&self) -> u32 {
        self.message_count.load(Ordering::Relaxed)
    }

    /// Get ICE restart offer count
    pub fn get_ice_restart_count(&self) -> u32 {
        self.ice_restart_offer_count.load(Ordering::SeqCst)
    }

    /// Get connection count
    pub fn get_connection_count(&self) -> u32 {
        self.connection_count.load(Ordering::SeqCst)
    }

    /// Get disconnection count
    pub fn get_disconnection_count(&self) -> u32 {
        self.disconnection_count.load(Ordering::SeqCst)
    }

    /// Reset all counters
    pub fn reset_counters(&self) {
        self.message_count.store(0, Ordering::Relaxed);
        self.ice_restart_offer_count.store(0, Ordering::SeqCst);
        self.connection_count.store(0, Ordering::SeqCst);
        self.disconnection_count.store(0, Ordering::SeqCst);
    }

    /// Check if server is running
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Acquire)
    }
}

impl Drop for TestSignalingServer {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
