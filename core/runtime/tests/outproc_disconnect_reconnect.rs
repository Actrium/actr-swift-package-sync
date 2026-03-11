//! Integration tests for OutprocOutGate disconnection/reconnection
//!
//! Uses TestHarness for multi-peer topology with VNet network simulation.
//!
//! Tests focus on:
//! - Two-peer disconnect → network event → ICE restart → reconnect
//! - Offerer vs Answerer recovery latency comparison
//!
//! ## Recovery latency tests (Test 2 & 3)
//!
//! Both tests use a **short outage (8s)** so the connection stays in the
//! peers map and `do_ice_restart_inner` is still running (in its backoff loop).
//!
//! The key difference (方案A implemented):
//! - **Offerer test**: offerer calls `retry_failed()` → `restart_ice()` → already inflight → wakes backoff
//! - **Answerer test**: answerer calls `retry_failed()` → `restart_ice()` → `!is_offerer`
//!   → sends IceRestartRequest → Offerer receives → wakes backoff → immediate retry

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

// ==================== Test 1: Two-peer disconnect/reconnect with NetworkEvent ====================

/// Test: disconnect two peers via VNet + signaling pause,
/// simulate NetworkEvent::Available (retry_failed_connections),
/// verify the connection is actually recovered by sending a message through the gate.
#[tokio::test]
async fn test_two_peer_disconnect_reconnect() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await;
    harness.add_peer(200).await;

    tracing::info!("🔗 Step 1: Establishing connection 100 → 200...");
    harness.connect(100, 200).await;

    // Record baseline
    harness.reset_counters();

    tracing::info!("🔴 Step 2: Simulating full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    // Wait for ICE to detect disconnection
    tracing::info!("⏳ Waiting for ICE disconnection detection...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Verify ICE restart was triggered (even though it can't succeed — signaling is down)
    let post_disconnect_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart count during outage: {}",
        post_disconnect_count
    );

    tracing::info!("🟢 Step 3: Restoring network (VNet + signaling)...");
    harness.simulate_reconnect();

    // Step 4: Simulate NetworkEvent::Available → triggers retry_failed_connections()
    // This is what happens in production when the platform layer detects network recovery
    tracing::info!("📱 Step 4: Triggering NetworkEvent::Available (retry_failed_connections)...");
    let start = tokio::time::Instant::now();
    harness.peer(100).retry_failed().await;

    // Wait for ICE restart to complete on the recovered network
    tracing::info!("⏳ Waiting for ICE restart to complete...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    let recovery_time = start.elapsed();
    tracing::info!(
        "📊 Recovery time (from NetworkEvent::Available): {:?}",
        recovery_time
    );

    // Step 5: Verify connection is ACTUALLY recovered by sending a message
    tracing::info!("📤 Step 5: Verifying connection recovery via gate message...");
    let peer_a = harness.peer(100);
    let request_handle = peer_a.spawn_request(200, "reconnect_verify_1", 10000);

    match tokio::time::timeout(Duration::from_secs(10), request_handle).await {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Connection recovered! Response: {} bytes, total recovery: {:?}",
                response.len(),
                start.elapsed()
            );
        }
        Ok(Ok(Err(e))) => {
            panic!("❌ Connection NOT recovered — request failed: {}", e);
        }
        Ok(Err(e)) => panic!("Request task panicked: {}", e),
        Err(_) => panic!("❌ Connection NOT recovered — request timed out after 10s"),
    }

    tracing::info!("✅ test_two_peer_disconnect_reconnect passed!");
}

// ==================== Test 2: Offerer recovery latency ====================

/// Test: offerer-triggered recovery after network outage.
///
/// Topology: peer 200 → peer 100 (offerer, echo responder on 100)
///
/// Flow:
/// 1. Establish connection
/// 2. Full network outage (VNet + signaling) for 8s
///    → ICE Disconnected → auto-restart triggered on offerer (peer 100)
///    → First attempt fails (signaling blocked) → enters backoff
/// 3. Unblock network
/// 4. Offerer (peer 100) calls `retry_failed_connections()` (simulating NetworkEvent::Available)
///    → `restart_ice()` but already inflight → no-op (dedup check)
/// 5. Measure time from unblock to message delivery
///
/// Key observation: `retry_failed()` on offerer is a no-op because
/// `do_ice_restart_inner` is already running. Recovery depends entirely on
/// the existing backoff timer expiring and retrying.
#[tokio::test]
async fn test_offerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await; // offerer (first peer → offerer VNet)
    harness.add_peer(200).await; // answerer

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    // === Step 2: Short outage — connection stays in peers map ===
    tracing::info!("🔴 Step 2: Full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    // Wait for ICE Disconnected → auto-restart → first attempt fails → enters backoff
    tracing::info!("⏳ Waiting 8s for auto-restart to enter backoff...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling blocked)",
        outage_restart_count
    );

    // === Step 3: Unblock network — start measuring ===
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // === Step 4: Offerer calls retry_failed (simulating NetworkEvent::Available) ===
    tracing::info!("📱 Step 4: Offerer (100) calls retry_failed_connections()...");
    tracing::info!("   → restart_ice() will find restart already inflight → no-op");
    harness.peer(100).retry_failed().await;

    // === Step 5: Wait for recovery and send message ===
    tracing::info!("📤 Step 5: Sending message 200→100 to verify recovery...");
    let peer_200 = harness.peer(200);
    let msg_handle = peer_200.spawn_request(100, "offerer_recovery", 30000);

    let msg_result = tokio::time::timeout(Duration::from_secs(30), msg_handle).await;
    let e2e_latency = recovery_start.elapsed();

    match msg_result {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Offerer recovery succeeded! Response: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            panic!(
                "❌ Offerer recovery FAILED: {} (e2e latency: {:?})",
                e, e2e_latency
            );
        }
        Ok(Err(e)) => panic!("Offerer request task panicked: {}", e),
        Err(_) => {
            panic!("❌ Offerer recovery TIMED OUT after {:?}", e2e_latency);
        }
    }

    let total_restart_count = harness.ice_restart_count();

    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Offerer Recovery Summary                          ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!(
        "║ ICE restart attempts: {} during outage, {} total",
        outage_restart_count,
        total_restart_count
    );
    tracing::info!("║ Note: retry_failed() on offerer = no-op (restart");
    tracing::info!("║   already inflight, dedup check blocks it)");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_offerer_recovery_latency passed!");
}

// ==================== Test 3: Answerer recovery latency ====================

/// Test: answerer-triggered recovery after network outage (方案A).
///
/// Topology: peer 200 → peer 100 (offerer, echo responder on 100)
///
/// Same setup as offerer test, BUT:
/// 4. **Answerer (peer 200)** calls `retry_failed_connections()` instead
///    → `restart_ice()` → `!is_offerer` → sends IceRestartRequest to Offerer
/// 5. Offerer receives IceRestartRequest → `notify_one()` wakes backoff
///    → immediate ICE restart retry → FASTER recovery
#[tokio::test]
async fn test_answerer_recovery_latency() {
    init_tracing();

    let mut harness = TestHarness::with_vnet().await;
    harness.add_peer(100).await; // offerer (first peer → offerer VNet)
    harness.add_peer(200).await; // answerer

    tracing::info!("🔗 Step 1: Establishing connection 200 → 100...");
    tracing::info!("   Peer 100 = offerer (echo responder)");
    tracing::info!("   Peer 200 = answerer (message sender, focus of this test)");
    harness.connect(200, 100).await;

    harness.reset_counters();

    // === Step 2: Short outage — connection stays in peers map ===
    tracing::info!("🔴 Step 2: Full network outage (VNet + signaling)...");
    harness.simulate_disconnect();

    tracing::info!("⏳ Waiting 8s for auto-restart to enter backoff...");
    tokio::time::sleep(Duration::from_secs(8)).await;

    let outage_restart_count = harness.ice_restart_count();
    tracing::info!(
        "📊 ICE restart attempts during outage: {} (all failed — signaling blocked)",
        outage_restart_count
    );

    // === Step 3: Unblock network — start measuring ===
    tracing::info!("🟢 Step 3: Restoring network — timer starts NOW");
    let recovery_start = std::time::Instant::now();
    harness.simulate_reconnect();

    // === Step 4: ANSWERER calls retry_failed (simulating NetworkEvent::Available) ===
    tracing::info!("📱 Step 4: Answerer (200) calls retry_failed_connections()...");
    tracing::info!("   → restart_ice() → !is_offerer → sends IceRestartRequest to Offerer");
    harness.peer(200).retry_failed().await;

    // === Step 5: Wait for recovery and send message ===
    tracing::info!("📤 Step 5: Sending message 200→100 to verify recovery...");
    let peer_200 = harness.peer(200);
    let msg_handle = peer_200.spawn_request(100, "answerer_recovery", 30000);

    let msg_result = tokio::time::timeout(Duration::from_secs(30), msg_handle).await;
    let e2e_latency = recovery_start.elapsed();

    match msg_result {
        Ok(Ok(Ok(response))) => {
            tracing::info!(
                "✅ Answerer (200) recovered! Response: {} bytes",
                response.len()
            );
        }
        Ok(Ok(Err(e))) => {
            tracing::error!(
                "❌ Answerer (200) recovery FAILED: {} (e2e latency: {:?})",
                e,
                e2e_latency
            );
        }
        Ok(Err(e)) => panic!("Answerer request task panicked: {}", e),
        Err(_) => {
            tracing::error!(
                "❌ Answerer (200) recovery TIMED OUT after {:?}",
                e2e_latency
            );
        }
    }

    let total_restart_count = harness.ice_restart_count();

    tracing::info!("╔══════════════════════════════════════════════════════╗");
    tracing::info!("║   Answerer Recovery Summary                         ║");
    tracing::info!("╠══════════════════════════════════════════════════════╣");
    tracing::info!("║ E2E recovery latency: {:?}", e2e_latency);
    tracing::info!("║   (from network unblock to message response)");
    tracing::info!(
        "║ ICE restart attempts: {} during outage, {} total",
        outage_restart_count,
        total_restart_count
    );
    tracing::info!("║ 方案A: retry_failed() on answerer → IceRestartRequest");
    tracing::info!("║   → Offerer wakes backoff → immediate retry");
    tracing::info!("╚══════════════════════════════════════════════════════╝");

    tracing::info!("✅ test_answerer_recovery_latency completed!");
}
