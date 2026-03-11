//! Common test utilities for WebRTC and signaling tests
//!
//! This module provides shared infrastructure for integration tests:
//! - WebSocket-based signaling server (TestSignalingServer)
//! - Helper functions for creating peers and credentials
//! - Virtual network (VNet) for simulating network disconnection
//! - Common test utilities

pub mod harness;
pub mod signaling;
pub mod utils;
pub mod vnet;

pub use harness::{TestHarness, TestPeer};
pub use signaling::TestSignalingServer;
pub use utils::*;
pub use vnet::{VNetPair, create_vnet_pair};
