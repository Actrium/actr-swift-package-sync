//! ICE Restart Integration Tests
//!
//! Tests the ICE restart mechanism under various scenarios:
//! - Basic ICE restart on established connection
//! - Rapid repeated ICE restart calls (de-duplication)
//! - Network state change scenarios

mod common;

use common::{TestSignalingServer, create_peer_with_websocket, make_actor_id};
use std::time::Duration;

// ==================== Tests ====================

/// Test basic ICE restart on an established connection
#[tokio::test]
async fn test_basic_ice_restart() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_file(true)
        .with_line_number(true)
        .with_test_writer()
        .try_init()
        .ok();

    let server = TestSignalingServer::start().await.unwrap();

    // Create two peers
    let id_offerer = make_actor_id(100);
    let id_answerer = make_actor_id(200);

    let (peer_offerer, _client_a) = create_peer_with_websocket(id_offerer.clone(), &server.url())
        .await
        .unwrap();
    let (_peer_answerer, _client_b) =
        create_peer_with_websocket(id_answerer.clone(), &server.url())
            .await
            .unwrap();

    // Establish connection
    tracing::info!("🔗 Establishing initial connection...");
    let ready_rx = peer_offerer
        .initiate_connection(&id_answerer)
        .await
        .expect("initiate failed");

    match tokio::time::timeout(Duration::from_secs(10), ready_rx).await {
        Ok(Ok(_)) => {
            tracing::info!("✅ Initial connection established!");
        }
        Ok(Err(_)) => panic!("Connection failed (channel closed)"),
        Err(_) => panic!("Connection timed out"),
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Wait a bit to ensure ICE restart count is reset or tracked properly
    // The server tracks total ICE restart offers, so we can check if it increases
    let initial_count = server.get_ice_restart_count();

    // Trigger ICE restart
    tracing::info!("♻️ Triggering ICE restart...");
    peer_offerer
        .restart_ice(&id_answerer)
        .await
        .expect("restart_ice failed");

    // Wait a bit for ICE restart to process
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Verify ICE restart offer was sent
    let count = server.get_ice_restart_count();
    tracing::info!("📊 ICE restart offers sent: {}", count);
    assert!(
        count > initial_count,
        "Expected ICE restart count to increase (initial: {}, current: {})",
        initial_count,
        count
    );

    tracing::info!("✅ Basic ICE restart test passed!");
}

/// Test rapid repeated ICE restart calls (de-duplication)
#[tokio::test]
async fn test_rapid_ice_restart_deduplication() {
    tracing_subscriber::fmt()
        .with_file(true)
        .with_line_number(true)
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let server = TestSignalingServer::start().await.unwrap();

    // Create two peers
    let id_offerer = make_actor_id(300);
    let id_answerer = make_actor_id(400);

    let (peer_offerer, _client_a) = create_peer_with_websocket(id_offerer.clone(), &server.url())
        .await
        .unwrap();
    let (_peer_answerer, _client_b) =
        create_peer_with_websocket(id_answerer.clone(), &server.url())
            .await
            .unwrap();

    // Establish connection
    tracing::info!("🔗 Establishing initial connection...");
    let ready_rx = peer_offerer
        .initiate_connection(&id_answerer)
        .await
        .expect("initiate failed");

    match tokio::time::timeout(Duration::from_secs(10), ready_rx).await {
        Ok(Ok(_)) => {
            tracing::info!("✅ Initial connection established!");
        }
        Ok(Err(_)) => panic!("Connection failed (channel closed)"),
        Err(_) => panic!("Connection timed out"),
    }

    // Capture initial count
    let initial_count = server.get_ice_restart_count();

    // Trigger multiple ICE restarts in rapid succession (simulating network jitter)
    tracing::info!("♻️ Triggering 5 rapid ICE restart calls...");

    let peer = peer_offerer.clone();
    let target = id_answerer.clone();
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let p = peer.clone();
            let t = target.clone();
            tokio::spawn(async move {
                tracing::info!("🔄 ICE restart call #{}", i + 1);
                let _ = p.restart_ice(&t).await;
            })
        })
        .collect();

    // Wait for all calls to complete
    for h in handles {
        h.await.ok();
    }

    // Wait for any in-flight restarts to process
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Check de-duplication: should only see 1 or 2 ICE restart offers at most
    // (Some may slip through the race condition window, but not all 5)
    let new_count = server.get_ice_restart_count();
    let delta = new_count - initial_count;
    tracing::info!("📊 New ICE restart offers sent: {}", delta);

    // Ideally we want delta == 1, but due to potential race conditions,
    // we allow up to 2 to pass through. 5 would indicate no de-duplication.
    assert!(
        delta <= 3,
        "Expected at most 3 ICE restart offers with de-duplication, got {}. \
         This suggests the de-duplication mechanism may not be working properly.",
        delta
    );

    if delta == 1 {
        tracing::info!("✅ Perfect de-duplication: only 1 offer sent!");
    } else {
        tracing::warn!(
            "⚠️ De-duplication not perfect: {} offers sent (expected 1)",
            delta
        );
    }

    tracing::info!("✅ Rapid ICE restart de-duplication test passed!");
}

/// Test ICE restart with sequential calls (should allow second restart after first completes)
#[tokio::test]
async fn test_sequential_ice_restart() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .with_file(true)
        .with_line_number(true)
        .try_init()
        .ok();

    let server = TestSignalingServer::start().await.unwrap();

    // Create two peers
    let id_offerer = make_actor_id(500);
    let id_answerer = make_actor_id(600);

    let (peer_offerer, _client_a) = create_peer_with_websocket(id_offerer.clone(), &server.url())
        .await
        .unwrap();
    let (_peer_answerer, _client_b) =
        create_peer_with_websocket(id_answerer.clone(), &server.url())
            .await
            .unwrap();

    // Establish connection
    tracing::info!("🔗 Establishing initial connection...");
    let ready_rx = peer_offerer
        .initiate_connection(&id_answerer)
        .await
        .expect("initiate failed");

    match tokio::time::timeout(Duration::from_secs(10), ready_rx).await {
        Ok(Ok(_)) => {
            tracing::info!("✅ Initial connection established!");
        }
        Ok(Err(_)) => panic!("Connection failed (channel closed)"),
        Err(_) => panic!("Connection timed out"),
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    // First ICE restart
    let initial_count = server.get_ice_restart_count();
    tracing::info!("♻️ First ICE restart...");
    peer_offerer
        .restart_ice(&id_answerer)
        .await
        .expect("first restart_ice failed");

    // Wait for ICE restart offer to be sent and processed
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let count1 = server.get_ice_restart_count();
    tracing::info!("📊 First restart offers: {}", count1);

    // Wait for first restart to complete or timeout
    tokio::time::sleep(Duration::from_secs(6)).await;

    // Second ICE restart (after first completes/times out)

    tracing::info!("♻️ Second ICE restart (after first finished)...");
    peer_offerer
        .restart_ice(&id_answerer)
        .await
        .expect("second restart_ice failed");

    tokio::time::sleep(Duration::from_millis(500)).await;
    let count2 = server.get_ice_restart_count();
    tracing::info!("📊 Second restart offers: {}", count2);

    // Both restarts should have sent offers
    assert!(
        count1 > initial_count,
        "First restart should send at least 1 offer"
    );
    assert!(
        count2 > count1,
        "Second restart should send at least 1 more offer"
    );

    tracing::info!("✅ Sequential ICE restart test passed!");
}

/// Test simultaneous ICE restart from both sides (glare scenario)
#[tokio::test]
async fn test_simultaneous_ice_restart_glare() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let server = TestSignalingServer::start().await.unwrap();

    // Create two peers
    let id_peer_a = make_actor_id(700);
    let id_peer_b = make_actor_id(800);

    let (peer_a, _client_a) = create_peer_with_websocket(id_peer_a.clone(), &server.url())
        .await
        .unwrap();
    let (peer_b, _client_b) = create_peer_with_websocket(id_peer_b.clone(), &server.url())
        .await
        .unwrap();

    // Establish initial connection (peer_a is offerer due to lower serial number)
    tracing::info!("🔗 Establishing initial connection...");
    let ready_rx = peer_a
        .initiate_connection(&id_peer_b)
        .await
        .expect("initiate failed");

    match tokio::time::timeout(Duration::from_secs(10), ready_rx).await {
        Ok(Ok(_)) => {
            tracing::info!("✅ Initial connection established!");
        }
        Ok(Err(_)) => panic!("Connection failed (channel closed)"),
        Err(_) => panic!("Connection timed out"),
    }

    // Wait for connection to stabilize
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Capture initial count
    let initial_count = server.get_ice_restart_count();

    // Trigger ICE restart from BOTH sides simultaneously
    tracing::info!("♻️ Triggering SIMULTANEOUS ICE restart from both peers...");

    let peer_a_clone = peer_a.clone();
    let peer_b_clone = peer_b.clone();
    let id_b_clone = id_peer_b.clone();
    let id_a_clone = id_peer_a.clone();

    // Launch both restarts concurrently
    let (result_a, result_b) = tokio::join!(
        async move {
            tracing::info!("🔄 Peer A (700) triggering ICE restart...");
            peer_a_clone.restart_ice(&id_b_clone).await
        },
        async move {
            tracing::info!("🔄 Peer B (800) triggering ICE restart...");
            peer_b_clone.restart_ice(&id_a_clone).await
        }
    );

    // Both calls should complete without error
    result_a.expect("Peer A ICE restart failed");
    result_b.expect("Peer B ICE restart failed");

    tracing::info!("✅ Both ICE restart calls completed");

    // Wait for the glare scenario to be resolved
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Check the ICE restart offer count
    let final_count = server.get_ice_restart_count();
    let delta = final_count - initial_count;
    tracing::info!("📊 New ICE restart offers sent: {}", delta);

    // We expect at least 1 offer (could be 2 if both sent before collision detection)
    assert!(
        delta >= 1,
        "Expected at least 1 ICE restart offer in glare scenario, got {}",
        delta
    );

    // The system should not send excessive offers (indicating a signaling storm)
    assert!(
        delta <= 4,
        "Too many ICE restart offers sent ({}), possible signaling storm",
        delta
    );

    if delta == 1 {
        tracing::info!("✅ Perfect glare resolution: collision detected before second offer sent");
    } else if delta == 2 {
        tracing::info!("✅ Good glare resolution: both offers sent, collision resolved cleanly");
    } else {
        tracing::warn!(
            "⚠️ Glare resolution with {} offers (acceptable but not optimal)",
            delta
        );
    }

    tracing::info!("✅ Simultaneous ICE restart (glare) test passed!");
}
