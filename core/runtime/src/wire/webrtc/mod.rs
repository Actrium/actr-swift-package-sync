//! WebRTC subsystem
//!
//! Complete WebRTC P2P ConnectManage， package include ：
//! - signaling protocol quotient （Offer/Answer/ICE）
//! - Connect build independent andManage
//! - OutboundGate Implementation

pub mod connection; // WebRtcConnection Implementation
pub mod coordinator;
pub mod gate;
pub mod negotiator;
pub mod signaling;
pub(crate) mod trace;

// Re-export core center Type
pub use connection::WebRtcConnection;
pub use coordinator::WebRtcCoordinator;
pub use gate::WebRtcGate;
pub use negotiator::{IceServer, IceTransportPolicy, WebRtcConfig, WebRtcNegotiator};
pub use signaling::{
    AuthConfig, AuthType, ConnectionState, DisconnectReason, ReconnectConfig, SignalingClient,
    SignalingConfig, SignalingEvent, SignalingStats, WebSocketSignalingClient,
};
