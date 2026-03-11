//! Wire Layer 0: Physical wire layer
//!
//! Low-level transport implementations:
//! - webrtc: WebRTC transport (DataChannel, MediaTrack, Coordinator, Signaling)
//! - websocket: WebSocket transport
//!
//! **Note**: For intra-process communication, use `crate::transport::InprocTransportManager`

pub mod webrtc;
pub mod websocket;

// Re-export commonly used types
pub use webrtc::{
    AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebRtcConfig, WebRtcConnection, WebRtcCoordinator, WebRtcGate,
    WebRtcNegotiator, WebSocketSignalingClient,
};
pub use websocket::{WebSocketConnection, WebSocketGate, WebSocketServer, WsAuthContext};
