//! Wire Handle - Unified handle for Wire layer components
//!
//! Provides unified access interface to Wire layer components (WebRtcConnection/WebSocketConnection)
//! Uses enum dispatch for zero-cost abstraction

use super::error::NetworkResult;
use super::lane::DataLane;
use actr_protocol::PayloadType;

// Re-export Wire layer connection types
pub use crate::wire::webrtc::WebRtcConnection;
pub use crate::wire::websocket::WebSocketConnection;

/// WireHandle - Unified handle for Wire layer components
///
/// # Design Philosophy
/// - Uses enum dispatch instead of trait objects, achieving zero virtual call overhead
/// - Provides unified interface to access different Wire layer implementations
/// - Supports connection priority comparison (WebRTC > WebSocket)
#[derive(Clone, Debug)]
pub enum WireHandle {
    /// WebSocket connection handle
    WebSocket(WebSocketConnection),

    /// WebRTC connection handle
    WebRTC(WebRtcConnection),
}

impl WireHandle {
    /// Get connection type name
    #[inline]
    pub fn connection_type(&self) -> &'static str {
        match self {
            WireHandle::WebSocket(_) => "WebSocket",
            WireHandle::WebRTC(_) => "WebRTC",
        }
    }

    /// Connection priority (higher number = higher priority)
    #[inline]
    pub fn priority(&self) -> u8 {
        match self {
            WireHandle::WebSocket(_) => 0,
            WireHandle::WebRTC(_) => 1, // WebRTC has higher priority
        }
    }

    /// Establish connection
    #[inline]
    pub async fn connect(&self) -> NetworkResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.connect().await,
            WireHandle::WebRTC(rtc) => rtc.connect().await,
        }
    }

    /// Check if connected
    #[inline]
    pub fn is_connected(&self) -> bool {
        match self {
            WireHandle::WebSocket(ws) => ws.is_connected(),
            WireHandle::WebRTC(rtc) => rtc.is_connected(),
        }
    }

    /// Close connection
    #[inline]
    pub async fn close(&self) -> NetworkResult<()> {
        match self {
            WireHandle::WebSocket(ws) => ws.close().await,
            WireHandle::WebRTC(rtc) => rtc.close().await,
        }
    }

    /// Get or create DataLane (with caching)
    #[inline]
    pub async fn get_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        match self {
            WireHandle::WebSocket(ws) => ws.get_lane(payload_type).await,
            WireHandle::WebRTC(rtc) => rtc.get_lane(payload_type).await,
        }
    }

    /// Create Lane (backward compatible)
    #[inline]
    pub async fn create_lane(&self, payload_type: PayloadType) -> NetworkResult<DataLane> {
        self.get_lane(payload_type).await
    }

    /// Convert to WebRTC connection (if it is one)
    #[inline]
    pub fn as_webrtc(&self) -> Option<&WebRtcConnection> {
        match self {
            WireHandle::WebRTC(rtc) => Some(rtc),
            _ => None,
        }
    }

    /// Convert to WebSocket connection (if it is one)
    #[inline]
    pub fn as_websocket(&self) -> Option<&WebSocketConnection> {
        match self {
            WireHandle::WebSocket(ws) => Some(ws),
            _ => None,
        }
    }

    /// Invalidate cached lane for WebRTC connections (no-op for WebSocket).
    #[inline]
    pub async fn invalidate_lane(&self, payload_type: PayloadType) {
        if let WireHandle::WebRTC(rtc) = self {
            rtc.invalidate_lane(payload_type).await;
        }
    }
}

/// Wire connection status
#[derive(Debug, Clone)]
pub enum WireStatus {
    /// Connecting
    Connecting,

    /// Connection ready
    Ready(WireHandle),

    /// Connection failed
    Failed,
}
