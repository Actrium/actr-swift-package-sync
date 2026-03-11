//! PayloadType routing extension
//!
//! Provides static routing configuration and retry policy for PayloadType.

use actr_protocol::PayloadType;
use std::time::Duration;

/// Retry policy for a send operation.
///
/// Applies only to transient failures. Non-transient errors are returned immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum number of attempts (1 = no retry, 2 = one retry, etc.)
    pub max_attempts: u32,
    /// Initial backoff delay between attempts.
    pub initial_delay: Duration,
    /// Maximum backoff delay cap.
    pub max_delay: Duration,
}

/// DataChannel QoS configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataChannelQoS {
    /// Signaling: ordered, reliable
    Signal,

    /// Reliable: reliable transmission
    Reliable,

    /// Latency-first: allow packet loss
    LatencyFirst,
}

/// DataLane type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataLaneType {
    /// WebRTC DataChannel (with QoS)
    WebRtcDataChannel(DataChannelQoS),

    /// WebSocket
    WebSocket,
}

/// PayloadType routing extension
pub trait PayloadTypeExt {
    /// Get the list of supported DataLane types (ordered by priority)
    fn data_lane_types(self) -> &'static [DataLaneType];

    /// Retry policy for transient send failures.
    ///
    /// - `RpcSignal`: 1 retry (2 total attempts), 500 ms / 500 ms cap
    ///   (signals are time-sensitive; one fast retry, then give up)
    /// - `RpcReliable`: 4 retries (5 total attempts), 1 s initial / 5 s cap
    ///   (important messages; exponential backoff up to 5 s)
    /// - Stream / Media: no retry (caller owns flow control)
    fn retry_policy(self) -> RetryPolicy;
}

impl PayloadTypeExt for PayloadType {
    #[inline]
    fn retry_policy(self) -> RetryPolicy {
        match self {
            // Signals are time-sensitive: one fast retry only
            PayloadType::RpcSignal => RetryPolicy {
                max_attempts: 2,
                initial_delay: Duration::from_millis(500),
                max_delay: Duration::from_millis(500),
            },
            // Reliable RPC: up to 4 retries with exponential backoff
            PayloadType::RpcReliable => RetryPolicy {
                max_attempts: 5,
                initial_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(5),
            },
            // Stream and media: caller owns flow control; no framework retry
            PayloadType::StreamReliable
            | PayloadType::StreamLatencyFirst
            | PayloadType::MediaRtp => RetryPolicy {
                max_attempts: 1,
                initial_delay: Duration::ZERO,
                max_delay: Duration::ZERO,
            },
        }
    }

    #[inline]
    fn data_lane_types(self) -> &'static [DataLaneType] {
        match self {
            // RPC_RELIABLE - RpcEnvelope with reliable ordered transmission
            PayloadType::RpcReliable => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable),
                DataLaneType::WebSocket,
            ],

            // RPC_SIGNAL - RpcEnvelope with high-priority signaling channel
            PayloadType::RpcSignal => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Signal),
                DataLaneType::WebSocket,
            ],

            // STREAM_RELIABLE - DataStream with reliable ordered transmission
            PayloadType::StreamReliable => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable),
                DataLaneType::WebSocket,
            ],

            // STREAM_LATENCY_FIRST - DataStream with low latency partial-reliable transmission
            PayloadType::StreamLatencyFirst => &[
                DataLaneType::WebRtcDataChannel(DataChannelQoS::LatencyFirst),
                DataLaneType::WebSocket,
            ],

            // MEDIA_RTP - Not routed through DataLane, uses MediaFrameRegistry
            PayloadType::MediaRtp => &[],
        }
    }
}

impl DataLaneType {
    /// Determine if WebRTC connection is needed for this DataLane Type
    #[inline]
    pub fn needs_webrtc(self) -> bool {
        matches!(self, DataLaneType::WebRtcDataChannel(_))
    }

    /// Check if this DataLane Type supports WebSocket
    #[inline]
    pub fn supports_websocket(self) -> bool {
        matches!(self, DataLaneType::WebSocket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_signal_retry_policy() {
        let p = PayloadType::RpcSignal.retry_policy();
        assert_eq!(p.max_attempts, 2, "one retry only");
        assert_eq!(p.initial_delay, Duration::from_millis(500));
        assert_eq!(p.max_delay, Duration::from_millis(500));
    }

    #[test]
    fn rpc_reliable_retry_policy() {
        let p = PayloadType::RpcReliable.retry_policy();
        assert_eq!(p.max_attempts, 5, "four retries");
        assert_eq!(p.initial_delay, Duration::from_secs(1));
        assert_eq!(p.max_delay, Duration::from_secs(5));
    }

    #[test]
    fn stream_and_media_no_retry() {
        for pt in [
            PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst,
            PayloadType::MediaRtp,
        ] {
            let p = pt.retry_policy();
            assert_eq!(p.max_attempts, 1, "{pt:?} should have no retry");
        }
    }

    #[test]
    fn rpc_reliable_lane_types() {
        let lanes = PayloadType::RpcReliable.data_lane_types();
        assert!(lanes.contains(&DataLaneType::WebRtcDataChannel(DataChannelQoS::Reliable)));
        assert!(lanes.contains(&DataLaneType::WebSocket));
    }

    #[test]
    fn rpc_signal_lane_types() {
        let lanes = PayloadType::RpcSignal.data_lane_types();
        assert!(lanes.contains(&DataLaneType::WebRtcDataChannel(DataChannelQoS::Signal)));
    }

    #[test]
    fn media_rtp_has_no_lane() {
        assert!(PayloadType::MediaRtp.data_lane_types().is_empty());
    }
}
