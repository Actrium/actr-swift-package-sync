//! Virtual Network (VNet) test utilities
//!
//! Provides a controllable virtual network layer for integration tests,
//! enabling simulation of network disconnection, reconnection, and partitions
//! at the UDP transport level (below ICE).
//!
//! ## Architecture
//!
//! ```text
//!                 ┌──────────────────────────────┐
//!                 │        wan (Router)           │
//!                 │       192.168.0.0/24          │
//!                 │                               │
//!                 │  ┌─ add_chunk_filter ──────┐  │
//!                 │  │ network_blocked flag     │  │
//!                 │  │ controls packet drop     │  │
//!                 │  └─────────────────────────┘  │
//!                 │                               │
//!                 └──────┬──────────────────┬─────┘
//!                        │                  │
//!              ┌─────────▼──┐        ┌──────▼───────┐
//!              │ net_offerer │        │ net_answerer │
//!              │ (VNet/Net) │        │ (VNet/Net)   │
//!              └─────┬──────┘        └──────┬───────┘
//!                    │                      │
//!              SettingEngine           SettingEngine
//!              set_vnet(net)           set_vnet(net)
//!                    │                      │
//!              RTCPeerConnection      RTCPeerConnection
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! let vnet_pair = VNetPair::new().await?;
//!
//! // Use in RTCPeerConnection creation:
//! // setting_engine.set_vnet(Some(vnet_pair.net_offerer.clone()));
//!
//! // Block network traffic (simulate disconnection)
//! vnet_pair.block_network();
//!
//! // Unblock network (simulate recovery)
//! vnet_pair.unblock_network();
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use webrtc_util::vnet::net::{Net, NetConfig};
use webrtc_util::vnet::router::{Router, RouterConfig};

/// A pair of virtual networks connected through a shared router.
///
/// The router has a chunk filter that can be toggled to simulate
/// network disconnection/reconnection at the UDP level.
pub struct VNetPair {
    /// Virtual network for the Offerer — pass to `SettingEngine::set_vnet()`
    pub net_offerer: Arc<Net>,
    /// Virtual network for the Answerer — pass to `SettingEngine::set_vnet()`
    pub net_answerer: Arc<Net>,
    /// The shared WAN router connecting both networks
    pub wan: Arc<Mutex<Router>>,
    /// Flag controlling whether the network is blocked.
    /// When true, ALL UDP packets traversing the router are dropped.
    network_blocked: Arc<AtomicBool>,
}

impl VNetPair {
    /// Create a new VNet pair with a shared router.
    ///
    /// Both virtual networks are assigned IPs in the 192.168.0.0/24 subnet:
    /// - Offerer: 192.168.0.1
    /// - Answerer: 192.168.0.2
    ///
    /// A chunk filter is installed on the router that drops ALL packets
    /// when `network_blocked` is set to `true`.
    pub async fn new() -> anyhow::Result<Self> {
        let network_blocked = Arc::new(AtomicBool::new(false));

        // Create WAN router
        let wan = Router::new(RouterConfig {
            cidr: "192.168.0.0/24".to_string(),
            ..Default::default()
        })?;
        let wan = Arc::new(Mutex::new(wan));

        // Create virtual networks with static IPs
        let net_offerer = Net::new(Some(NetConfig {
            static_ips: vec!["192.168.0.1".to_string()],
            ..Default::default()
        }));

        let net_answerer = Net::new(Some(NetConfig {
            static_ips: vec!["192.168.0.2".to_string()],
            ..Default::default()
        }));

        // Get NIC handles
        let offerer_nic = net_offerer.get_nic()?;
        let answerer_nic = net_answerer.get_nic()?;

        // Register NICs with the router (needs &mut Router → lock)
        {
            let mut w = wan.lock().await;
            w.add_net(offerer_nic.clone()).await?;
            w.add_net(answerer_nic.clone()).await?;
        }
        // Lock released here

        // Set router reference on each NIC (needs wan Arc → can't hold Mutex above)
        {
            let nic = offerer_nic.lock().await;
            nic.set_router(wan.clone()).await?;
        }
        {
            let nic = answerer_nic.lock().await;
            nic.set_router(wan.clone()).await?;
        }

        // Install chunk filter for network blocking
        let blocked = network_blocked.clone();
        {
            let w = wan.lock().await;
            w.add_chunk_filter(Box::new(move |_chunk| {
                // Return false → drop packet, true → forward
                !blocked.load(Ordering::SeqCst)
            }))
            .await;
        }

        // Start the router
        {
            let mut w = wan.lock().await;
            w.start().await?;
        }

        tracing::info!("🌐 VNet pair created: offerer=192.168.0.1, answerer=192.168.0.2");

        Ok(Self {
            net_offerer: Arc::new(net_offerer),
            net_answerer: Arc::new(net_answerer),
            wan,
            network_blocked,
        })
    }

    /// Block all UDP traffic between the two networks.
    ///
    /// This simulates a complete network disconnection at the transport layer.
    /// ICE will detect connectivity loss and eventually transition to
    /// `Disconnected` → `Failed` state.
    pub fn block_network(&self) {
        tracing::warn!("🔴 VNet: Blocking ALL network traffic");
        self.network_blocked.store(true, Ordering::SeqCst);
    }

    /// Unblock UDP traffic between the two networks.
    ///
    /// This simulates network recovery. If an ICE restart is in progress,
    /// the Offerer's next retry will succeed after gathering new candidates.
    pub fn unblock_network(&self) {
        tracing::info!("🟢 VNet: Unblocking network traffic");
        self.network_blocked.store(false, Ordering::SeqCst);
    }

    /// Check if the network is currently blocked.
    pub fn is_blocked(&self) -> bool {
        self.network_blocked.load(Ordering::SeqCst)
    }
}

impl Drop for VNetPair {
    fn drop(&mut self) {
        tracing::debug!("🧹 VNet: Dropping VNetPair");
        // Router's channels will be dropped, signaling shutdown
    }
}

/// Convenience function to create a VNetPair.
pub async fn create_vnet_pair() -> anyhow::Result<VNetPair> {
    VNetPair::new().await
}
