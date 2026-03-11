//! Transport Layer 1: Transport layer
//!
//! Core Lane abstraction and transport management:
//! - Lane: Physical embodiment of PayloadType, unified bidirectional channel abstraction ⭐
//! - InprocTransportManager: Intra-process transport management (Workload ↔ Shell)
//! - OutprocTransportManager: Cross-process transport management (WebRTC + WebSocket)
//! - WireHandle: Unified handle for Wire layer components
//! - WirePool: Wire connection pool manager (strategy layer)
//! - WireBuilder: Wire layer component builder

mod backoff;
pub mod connection_event;
mod dest_transport;
pub mod error;
mod inproc_manager;
mod lane;
mod manager;
mod route_table;
mod wire_builder;
mod wire_handle;
mod wire_pool;

// Re-export Dest from actr-framework (unified API layer)
pub use actr_framework::Dest;

// DataLane core abstraction
pub use lane::DataLane;
pub use route_table::{DataChannelQoS, DataLaneType, PayloadTypeExt, RetryPolicy};

// Transport management
pub use inproc_manager::InprocTransportManager;
pub use manager::{OutprocTransportManager, WireBuilder};

// Backward compatible alias (deprecated)
pub use dest_transport::DestTransport;
#[deprecated(note = "Use OutprocTransportManager instead")]
pub use manager::OutprocTransportManager as TransportManager;

// Wire layer management
pub use wire_builder::{DefaultWireBuilder, DefaultWireBuilderConfig};
pub use wire_handle::WireHandle;
pub use wire_pool::WirePool;

// Error types
pub use error::{NetworkError, NetworkResult};

// Retry and backoff strategies
pub use backoff::ExponentialBackoff;

// Connection events
pub use connection_event::{ConnectionEvent, ConnectionEventBroadcaster, ConnectionState};

// Connection session
pub mod session;
pub use session::ConnectionSession;
