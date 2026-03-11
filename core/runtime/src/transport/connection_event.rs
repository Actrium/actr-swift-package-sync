//! Connection Event System
//!
//! Unified event broadcasting mechanism for connection state changes.
//! Enables proactive resource cleanup across all transport layers.

use actr_protocol::{ActrId, PayloadType};
use tokio::sync::broadcast;

/// Connection state enumeration
/// Maps to WebRTC RTCPeerConnectionState
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Initial state
    New,
    /// ICE/DTLS negotiation in progress
    Connecting,
    /// Connection established
    Connected,
    /// ICE connectivity lost (may recover)
    Disconnected,
    /// ICE connectivity failed (offerer should try ICE restart)
    Failed,
    /// Connection closed
    Closed,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionState::New => write!(f, "New"),
            ConnectionState::Connecting => write!(f, "Connecting"),
            ConnectionState::Connected => write!(f, "Connected"),
            ConnectionState::Disconnected => write!(f, "Disconnected"),
            ConnectionState::Failed => write!(f, "Failed"),
            ConnectionState::Closed => write!(f, "Closed"),
        }
    }
}

/// Connection events broadcast to all subscribers
#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    /// Connection state changed
    StateChanged {
        peer_id: ActrId,
        session_id: u64,
        state: ConnectionState,
    },

    /// DataChannel closed for specific payload type
    DataChannelClosed {
        peer_id: ActrId,
        session_id: u64,
        payload_type: PayloadType,
    },

    /// DataChannel opened for specific payload type
    /// This event is fired when a DataChannel transitions to Open state,
    /// indicating SCTP layer is ready for data transmission
    DataChannelOpened {
        peer_id: ActrId,
        session_id: u64,
        payload_type: PayloadType,
    },

    /// Connection fully closed (triggers full cleanup)
    ConnectionClosed { peer_id: ActrId, session_id: u64 },

    /// ICE restart started
    IceRestartStarted { peer_id: ActrId, session_id: u64 },

    /// ICE restart completed
    IceRestartCompleted {
        peer_id: ActrId,
        session_id: u64,
        success: bool,
    },

    /// New offer received (triggers cleanup of existing connection)
    NewOfferReceived { peer_id: ActrId, sdp: String },

    /// New role assignment (triggers cleanup if role changed)
    NewRoleAssignment { peer_id: ActrId, is_offerer: bool },
}

impl ConnectionEvent {
    /// Get the peer_id from the event
    pub fn peer_id(&self) -> &ActrId {
        match self {
            ConnectionEvent::StateChanged { peer_id, .. } => peer_id,
            ConnectionEvent::DataChannelClosed { peer_id, .. } => peer_id,
            ConnectionEvent::DataChannelOpened { peer_id, .. } => peer_id,
            ConnectionEvent::ConnectionClosed { peer_id, .. } => peer_id,
            ConnectionEvent::IceRestartStarted { peer_id, .. } => peer_id,
            ConnectionEvent::IceRestartCompleted { peer_id, .. } => peer_id,
            ConnectionEvent::NewOfferReceived { peer_id, .. } => peer_id,
            ConnectionEvent::NewRoleAssignment { peer_id, .. } => peer_id,
        }
    }

    /// Get the session_id from the event (None for events without session)
    pub fn session_id(&self) -> Option<u64> {
        match self {
            Self::StateChanged { session_id, .. }
            | Self::DataChannelClosed { session_id, .. }
            | Self::DataChannelOpened { session_id, .. }
            | Self::ConnectionClosed { session_id, .. }
            | Self::IceRestartStarted { session_id, .. }
            | Self::IceRestartCompleted { session_id, .. } => Some(*session_id),
            _ => None,
        }
    }

    /// Check if this event should trigger full resource cleanup
    pub fn should_trigger_cleanup(&self) -> bool {
        matches!(
            self,
            ConnectionEvent::ConnectionClosed { .. }
                | ConnectionEvent::StateChanged {
                    state: ConnectionState::Closed,
                    ..
                }
                | ConnectionEvent::IceRestartCompleted { success: false, .. }
        )
    }

    /// Check if this event indicates a recoverable state (ICE restart candidate)
    pub fn is_recoverable_state(&self) -> bool {
        matches!(
            self,
            ConnectionEvent::StateChanged {
                state: ConnectionState::Disconnected | ConnectionState::Failed,
                ..
            }
        )
    }
}

/// Default broadcast channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 256;

/// Connection event broadcaster
///
/// Manages a broadcast channel for distributing connection events
/// to all subscribed layers.
#[derive(Debug)]
pub struct ConnectionEventBroadcaster {
    tx: broadcast::Sender<ConnectionEvent>,
}

impl ConnectionEventBroadcaster {
    /// Create a new broadcaster with default capacity
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new broadcaster with specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Send an event to all subscribers
    ///
    /// Returns the number of receivers that received the event.
    /// Returns 0 if there are no active subscribers (not an error).
    pub fn send(&self, event: ConnectionEvent) -> usize {
        self.tx.send(event).unwrap_or_default()
    }

    /// Subscribe to connection events
    pub fn subscribe(&self) -> broadcast::Receiver<ConnectionEvent> {
        self.tx.subscribe()
    }

    /// Get the number of active subscribers
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Get a clone of the sender for sharing
    pub fn sender(&self) -> broadcast::Sender<ConnectionEvent> {
        self.tx.clone()
    }
}

impl Default for ConnectionEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ConnectionEventBroadcaster {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::{ActrId, ActrType, Realm};

    fn test_peer_id() -> ActrId {
        ActrId {
            realm: Realm { realm_id: 1 },
            serial_number: 1,
            r#type: ActrType {
                manufacturer: "test".to_string(),
                name: "device".to_string(),
                version: "v1".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_broadcaster_send_receive() {
        let broadcaster = ConnectionEventBroadcaster::new();
        let mut rx = broadcaster.subscribe();

        let peer_id = test_peer_id();
        broadcaster.send(ConnectionEvent::ConnectionClosed {
            peer_id: peer_id.clone(),
            session_id: 0,
        });

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, ConnectionEvent::ConnectionClosed { .. }));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let broadcaster = ConnectionEventBroadcaster::new();
        let mut rx1 = broadcaster.subscribe();
        let mut rx2 = broadcaster.subscribe();

        let peer_id = test_peer_id();
        let count = broadcaster.send(ConnectionEvent::StateChanged {
            peer_id: peer_id.clone(),
            session_id: 0,
            state: ConnectionState::Connected,
        });

        assert_eq!(count, 2);

        let event1 = rx1.recv().await.unwrap();
        let event2 = rx2.recv().await.unwrap();

        assert!(matches!(
            event1,
            ConnectionEvent::StateChanged {
                state: ConnectionState::Connected,
                ..
            }
        ));
        assert!(matches!(
            event2,
            ConnectionEvent::StateChanged {
                state: ConnectionState::Connected,
                ..
            }
        ));
    }

    #[test]
    fn test_should_trigger_cleanup() {
        let peer_id = test_peer_id();

        // Should trigger cleanup
        assert!(
            ConnectionEvent::ConnectionClosed {
                peer_id: peer_id.clone(),
                session_id: 0,
            }
            .should_trigger_cleanup()
        );

        assert!(
            ConnectionEvent::StateChanged {
                peer_id: peer_id.clone(),
                session_id: 0,
                state: ConnectionState::Closed,
            }
            .should_trigger_cleanup()
        );

        assert!(
            ConnectionEvent::IceRestartCompleted {
                peer_id: peer_id.clone(),
                session_id: 0,
                success: false,
            }
            .should_trigger_cleanup()
        );

        // Should NOT trigger cleanup
        assert!(
            !ConnectionEvent::StateChanged {
                peer_id: peer_id.clone(),
                session_id: 0,
                state: ConnectionState::Disconnected,
            }
            .should_trigger_cleanup()
        );

        assert!(
            !ConnectionEvent::IceRestartCompleted {
                peer_id: peer_id.clone(),
                session_id: 0,
                success: true,
            }
            .should_trigger_cleanup()
        );
    }

    #[test]
    fn test_is_recoverable_state() {
        let peer_id = test_peer_id();

        // Recoverable states
        assert!(
            ConnectionEvent::StateChanged {
                peer_id: peer_id.clone(),
                session_id: 0,
                state: ConnectionState::Disconnected,
            }
            .is_recoverable_state()
        );

        assert!(
            ConnectionEvent::StateChanged {
                peer_id: peer_id.clone(),
                session_id: 0,
                state: ConnectionState::Failed,
            }
            .is_recoverable_state()
        );

        // Not recoverable
        assert!(
            !ConnectionEvent::StateChanged {
                peer_id: peer_id.clone(),
                session_id: 0,
                state: ConnectionState::Closed,
            }
            .is_recoverable_state()
        );

        assert!(
            !ConnectionEvent::ConnectionClosed {
                peer_id: peer_id.clone(),
                session_id: 0,
            }
            .is_recoverable_state()
        );
    }
}
