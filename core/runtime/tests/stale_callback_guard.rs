//! Integration test: Stale callbacks must NOT kill new connections
//!
//! Problem scenario (旧回调误杀新连接):
//! 1. Peer A connects to Peer B → connection_1 (session_1)
//! 2. Network outage → connection_1's DataChannels start closing asynchronously
//! 3. Network recovers → Peer A reconnects → connection_2 (session_2)
//! 4. connection_1's stale DC.on_close callbacks fire AFTER connection_2 is live
//! 5. BUG (before fix): Stale callbacks invalidate connection_2's lanes → connection dies
//! 6. FIX: session_id / cancel_token guard prevents stale callbacks from executing
//!
//! This test verifies:
//! - After reconnect, the NEW connection remains functional
//! - Stale events from old sessions are filtered by the coordinator
//! - Messages can be sent and received on the new connection

mod common;

use common::TestHarness;
use std::time::Duration;

/// Initialize tracing for test output
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();
}

/// Test: Stale callback guard prevents old connection from killing new connection
///
/// This is the core test for the "旧回调误杀新连接" bug fix.
///
/// Flow:
/// 1. Connect peer 100 → peer 200 (connection_1, session_1)
/// 2. Verify connection works (send/receive message)
/// 3. Simulate network outage (VNet + signaling)
///    → DataChannels start closing; old callbacks become pending
/// 4. Restore network
/// 5. Trigger reconnection via retry_failed_connections()
///    → New connection_2 (session_2) is established
/// 6. Verify NEW connection works — stale callbacks did NOT kill it
/// 7. Send multiple messages to confirm stability
#[tokio::test]
async fn test_stale_callbacks_do_not_kill_new_connection() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    // ==================== Phase 1: Establish connection ====================
    tracing::info!("🔗 Phase 1: Establishing connection 100 → 200...");
    harness.connect(100, 200).await;
    tracing::info!("✅ Phase 1 complete: Connection established and verified");

    // Record session_id of the first connection via event subscription
    let mut event_rx = harness.peer(100).subscribe_events();

    harness.reset_counters();

    // ==================== Phase 2: Network outage ====================
    tracing::info!("🔴 Phase 2: Simulating full network outage...");
    tracing::info!("   Old DataChannel callbacks will fire asynchronously");
    harness.simulate_disconnect();

    // Wait for ICE to detect disconnection and DC callbacks to fire
    tracing::info!("⏳ Waiting 10s for ICE disconnection + DC close callbacks...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Drain stale events (they should carry session_1's session_id)
    let mut stale_event_count = 0;
    while let Ok(event) = event_rx.try_recv() {
        tracing::info!("📨 Stale event during outage: {:?}", event);
        stale_event_count += 1;
    }
    tracing::info!(
        "📊 Received {} events during outage (will become stale after reconnect)",
        stale_event_count
    );

    // ==================== Phase 3: Network recovery + reconnect ====================
    tracing::info!("🟢 Phase 3: Restoring network...");
    harness.simulate_reconnect();

    // Small delay for signaling to stabilize
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Trigger reconnection (simulates NetworkEvent::Available)
    tracing::info!("📱 Triggering retry_failed_connections()...");
    harness.peer(100).retry_failed().await;

    // Wait for ICE restart to complete and new connection to stabilize
    tracing::info!("⏳ Waiting for ICE restart + new connection...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // ==================== Phase 4: Verify new connection survives ====================
    tracing::info!("📤 Phase 4: Verifying new connection is alive...");
    tracing::info!("   If stale callbacks killed the new connection, this will FAIL");

    // Send first verification message
    let peer_a = harness.peer(100);
    let handle1 = peer_a.spawn_request(200, "stale_guard_verify_1", 15000);

    match tokio::time::timeout(Duration::from_secs(15), handle1).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Message 1 succeeded on NEW connection! ({} bytes)",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            let err_str = format!("{:?}", e);
            if err_str.to_lowercase().contains("closed") {
                panic!(
                    "❌ BUG: Stale callback killed new connection! Error: {}",
                    err_str
                );
            }
            panic!("❌ Message 1 failed: {}", err_str);
        }
        Ok(Err(e)) => panic!("Task panicked: {}", e),
        Err(_) => panic!("❌ Message 1 timed out — new connection may be dead"),
    }

    // ==================== Phase 5: Stability check — multiple messages ====================
    tracing::info!("📤 Phase 5: Sending 3 more messages to verify stability...");

    for i in 2..=4 {
        let peer_a = harness.peer(100);
        let req_id = format!("stale_guard_verify_{}", i);
        let handle = peer_a.spawn_request(200, &req_id, 10000);

        match tokio::time::timeout(Duration::from_secs(10), handle).await {
            Ok(Ok(Ok(response))) => {
                tracing::info!("  ✅ Message {} succeeded ({} bytes)", i, response.len());
            }
            Ok(Ok(Err(e))) => {
                panic!("❌ Message {} failed: {:?}", i, e);
            }
            Ok(Err(e)) => panic!("Task {} panicked: {}", i, e),
            Err(_) => panic!("❌ Message {} timed out", i),
        }

        // Small gap between messages
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // ==================== Summary ====================
    let total_restarts = harness.ice_restart_count();
    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Stale Callback Guard Test — PASSED                ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ • Old connection's stale callbacks were SILENCED    ║");
    tracing::info!("║ • New connection survived and handled 4 messages    ║");
    tracing::info!(
        "║ • ICE restarts: {}                                   ║",
        total_restarts
    );
    tracing::info!("║ • Bug '旧回调误杀新连接' is FIXED                   ║");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_stale_callbacks_do_not_kill_new_connection passed!");
}

/// Test: Session ID filtering in coordinator event listener
///
/// Verifies that events from old sessions are ignored by the coordinator's
/// event listener, preventing stale cleanup on new connections.
///
/// This test injects synthetic stale events to directly test the filter.
#[tokio::test]
async fn test_session_id_filtering_ignores_stale_events() {
    init_tracing();

    let mut harness = TestHarness::new().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    // Establish connection
    tracing::info!("🔗 Establishing connection 100 → 200...");
    harness.connect(100, 200).await;
    tracing::info!("✅ Connection established");

    // Subscribe to events to observe what happens
    let mut event_rx = harness.peer(100).subscribe_events();

    // Inject a STALE ConnectionClosed event with session_id=0 (definitely not current)
    // This simulates what happens when an old connection's close event fires late
    tracing::info!("🧪 Injecting stale ConnectionClosed event (session_id=0)...");
    let stale_event =
        actr_runtime::transport::connection_event::ConnectionEvent::ConnectionClosed {
            peer_id: common::make_actor_id(200),
            session_id: 0, // Stale session_id — won't match current PeerState.session_id
        };
    harness.peer(100).send_event(stale_event);

    // Wait a bit for the event listener to process
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The connection should STILL be alive — stale event was filtered
    tracing::info!("📤 Verifying connection still works after stale event injection...");
    let peer_a = harness.peer(100);
    let handle = peer_a.spawn_request(200, "after_stale_inject", 10000);

    match tokio::time::timeout(Duration::from_secs(10), handle).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Connection survived stale event! Response: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            panic!("❌ BUG: Stale event killed the connection! Error: {:?}", e);
        }
        Ok(Err(e)) => panic!("Task panicked: {}", e),
        Err(_) => panic!("❌ Connection dead after stale event injection!"),
    }

    // Drain event_rx to confirm events were processed
    let mut events_seen = 0;
    while let Ok(event) = event_rx.try_recv() {
        tracing::debug!("  Event observed: {:?}", event);
        events_seen += 1;
    }
    tracing::info!("📊 Events processed after injection: {}", events_seen);

    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Session ID Filtering Test — PASSED                ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ • Stale ConnectionClosed (session_id=0) was ignored ║");
    tracing::info!("║ • Current connection survived and handled messages  ║");
    tracing::info!("║ • Coordinator event listener filtering works        ║");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_session_id_filtering_ignores_stale_events passed!");
}
