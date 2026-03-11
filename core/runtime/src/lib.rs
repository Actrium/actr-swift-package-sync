//! # Actor-RTC Runtime Layer
//!
//! Runtime infrastructure for the Actor-RTC framework, providing complete Actor lifecycle management
//! and message transport capabilities.
//!
//! ## Overview
//!
//! `actr-runtime` is the core runtime library that powers Actor nodes. It provides:
//!
//! - **Actor Lifecycle**: System initialization, node startup/shutdown
//! - **Message Transport**: Multi-layer architecture (Wire → Transport → Gate → Dispatch)
//! - **Communication Modes**: Intra-process (zero-copy) and cross-process (WebRTC/WebSocket)
//! - **Message Persistence**: SQLite-backed mailbox with ACID guarantees
//!
//! ## Architecture Layers
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  Lifecycle Management (ActrSystem → ActrNode → ActrRef)
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 3: Inbound Dispatch                          │  DataStreamRegistry
//! │           (Fast Path Routing)                       │  MediaFrameRegistry
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 2: Outbound Gate                             │  InprocOutGate
//! │           (Message Sending)                         │  OutprocOutGate
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 1: Transport                                 │  Lane (core abstraction)
//! │           (Channel Management)                      │  InprocTransportManager
//! │                                                     │  OutprocTransportManager
//! ├─────────────────────────────────────────────────────┤
//! │  Layer 0: Wire                                      │  WebRtcGate
//! │           (Physical Connections)                    │  WebRtcCoordinator
//! │                                                     │  SignalingClient
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Core Types
//!
//! ### Lifecycle Management
//!
//! The runtime follows a three-phase lifecycle:
//!
//! ```rust,ignore
//! use actr_runtime::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> ActorResult<()> {
//!     // Phase 1: Create generic-free infrastructure
//!     let config = actr_config::Config::default();
//!     let system = ActrSystem::new(config).await?;
//!
//!     // Phase 2: Attach business logic (Workload)
//!     let node = system.attach(MyWorkload::default());
//!
//!     // Phase 3: Start and get running node
//!     let running = node.start().await?;
//!
//!     // Option 1: Convenience method (wait for Ctrl+C and shutdown)
//!     running.wait_for_ctrl_c_and_shutdown().await?;
//!
//!     // Option 2: Manual control (for custom shutdown logic)
//!     // tokio::signal::ctrl_c().await?;
//!     // running.shutdown().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Transport Abstraction
//!
//! **Lane** is the core transport abstraction, representing a unidirectional message channel:
//!
//! ```rust,ignore
//! use actr_runtime::transport::Lane;
//! use actr_protocol::PayloadType;
//!
//! // Lane supports 4 variants:
//! // - WebRtcDataChannel: Reliable/Signal/LatencyFirst
//! // - WebRtcMediaTrack: Real-time media
//! // - Mpsc: Intra-process (zero-copy)
//! // - WebSocket: Fallback transport
//!
//! // Send message through lane
//! lane.send(data).await?;
//!
//! // Receive from lane
//! let data = lane.recv().await?;
//! ```
//!
//! ### Communication Modes
//!
//! #### Intra-process (Shell ↔ Workload)
//!
//! ```rust,ignore
//! use actr_runtime::transport::InprocTransportManager;
//!
//! let mgr = InprocTransportManager::new();
//!
//! // Zero-serialization, direct RpcEnvelope passing
//! let response = mgr.send_request(
//!     PayloadType::RpcReliable,
//!     None,  // identifier
//!     envelope
//! ).await?;
//! ```
//!
//! #### Cross-process (WebRTC/WebSocket)
//!
//! ```rust,ignore
//! use actr_runtime::transport::OutprocTransportManager;
//!
//! let mgr = OutprocTransportManager::new(config);
//!
//! // Protobuf serialization, automatic protocol negotiation
//! mgr.send(&dest, PayloadType::RpcReliable, &data).await?;
//! ```
//!
//! ### Message Persistence
//!
//! All incoming messages go through the Mailbox (State Path):
//!
//! ```rust,ignore
//! use actr_runtime_mailbox::{Mailbox, MessagePriority};
//!
//! // Enqueue message (persisted to SQLite)
//! let msg_id = mailbox.enqueue(from, payload, MessagePriority::Normal).await?;
//!
//! // Dequeue batch (ordered by priority)
//! let messages = mailbox.dequeue().await?;
//!
//! // Process and acknowledge
//! for msg in messages {
//!     process_message(&msg).await?;
//!     mailbox.ack(msg.id).await?;
//! }
//! ```
//!
//! ## Feature Status
//!
//! ### ✅ Implemented
//!
//! - Actor lifecycle management (ActrSystem/ActrNode/ActrRef)
//! - 4-Lane transport architecture (WebRTC DataChannel + MediaTrack)
//! - WebSocket signaling client
//! - Intra-process zero-copy channels
//! - SQLite-backed mailbox with priorities
//! - Context factory for message handling
//! - Basic service discovery helpers (RouteCandidates)
//! - Distributed tracing integration (feature `opentelemetry`, Jaeger/OTLP)
//!
//! ### ⏳ Pending
//!
//! - Health checks and metrics collection
//! - Prometheus metrics export
//!
//! ## Usage Note
//!
//! This is a low-level runtime library. For application development, use the high-level
//! framework APIs provided by `actr-framework` which builds on top of this runtime.

// AIS signing 公钥缓存（按 key_id 索引，miss 时通过 signaling 拉取）
pub mod ais_key_cache;

// Lifecycle management layer (not architectural layering)
pub mod lifecycle;

// ActrRef - Lightweight Actor reference
pub mod actr_ref;

// Layer 3: Inbound dispatch layer
pub mod inbound;

// Layer 2: Outbound gate abstraction layer
pub mod outbound;

// Layer 1: Transport layer
pub mod transport;

// Layer 0: Wire layer
pub mod wire;

pub mod context;
pub mod context_factory;
pub mod error;
pub mod monitoring;
pub mod observability;
pub mod resource;

// TODO: Implement health check and metrics collection modules
// pub mod health;
// pub mod metrics;

pub use observability::{ObservabilityGuard, init_observability};

// Re-export core types
pub use actr_protocol::{ActrId, ActrType};

// Runtime core structures
pub use actr_ref::ActrRef;
pub use lifecycle::{ActrNode, ActrSystem, CredentialState, NetworkEventHandle};

// Layer 3: Inbound dispatch layer
pub use inbound::{DataStreamCallback, DataStreamRegistry, MediaFrameRegistry, MediaTrackCallback};

// Re-export MediaSample and MediaType from framework (dependency inversion)
pub use actr_framework::{MediaSample, MediaType};

// Layer 2: Outbound gate abstraction layer
pub use outbound::{InprocOutGate, OutGate, OutprocOutGate};

// Layer 1: Transport layer
pub use transport::{
    DataLane,
    DefaultWireBuilder,
    DefaultWireBuilderConfig,
    Dest,
    DestTransport,
    ExponentialBackoff,
    InprocTransportManager, // Intra-process transport manager
    NetworkError,
    NetworkResult,
    OutprocTransportManager,
    WireBuilder,
    WireHandle,
};

// Backward compatible alias (deprecated)
#[allow(deprecated)]
pub use transport::TransportManager;

// Layer 0: Wire Layer
pub use ais_key_cache::AisKeyCache;
pub use wire::{
    AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
    SignalingEvent, SignalingStats, WebRtcConfig, WebRtcCoordinator, WebRtcGate, WebRtcNegotiator,
    WebSocketConnection, WebSocketGate, WebSocketServer, WebSocketSignalingClient, WsAuthContext,
};

// Mailbox
pub use actr_runtime_mailbox::{
    Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
};

// System interfaces
pub use context_factory::ContextFactory;

// Utility modules
pub use error::{ActorResult, ActrError, Classify, ErrorKind};
pub use monitoring::{Alert, AlertConfig, AlertSeverity, Monitor, MonitoringConfig};
pub use resource::{ResourceConfig, ResourceManager, ResourceQuota, ResourceUsage};

// Optional feature re-exports (pending implementation)
// pub use health::{HealthChecker, HealthStatus, HealthReport};
// pub use metrics::{RuntimeMetrics, MetricsRegistry, MetricsConfig};

pub mod prelude {
    //! Convenience prelude module
    //!
    //! Re-exports commonly used types and traits for quick imports:
    //!
    //! ```rust
    //! use actr_runtime::prelude::*;
    //! ```

    // Core structures
    pub use crate::actr_ref::ActrRef;
    pub use crate::lifecycle::{
        ActrNode, ActrSystem, CompatLockFile, CompatLockManager, CompatibilityCheck,
        DiscoveryResult,
    };

    // Layer 3: Inbound dispatch layer
    pub use crate::inbound::{
        DataStreamCallback, DataStreamRegistry, MediaFrameRegistry, MediaTrackCallback,
    };

    // Re-export MediaSample and MediaType from framework (dependency inversion)
    pub use actr_framework::{MediaSample, MediaType};

    // Layer 2: Outbound gate abstraction layer
    pub use crate::outbound::{InprocOutGate, OutGate, OutprocOutGate};

    // System interfaces
    pub use crate::context_factory::ContextFactory;

    // WebRTC subsystem
    pub use crate::wire::webrtc::{
        AuthConfig, AuthType, IceServer, ReconnectConfig, SignalingClient, SignalingConfig,
        WebRtcConfig, WebRtcCoordinator, WebRtcGate, WebRtcNegotiator, WebSocketSignalingClient,
    };

    // Mailbox subsystem
    pub use actr_runtime_mailbox::{
        Mailbox, MailboxStats, MessagePriority, MessageRecord, MessageStatus,
    };

    // Transport layer (transport management)
    pub use crate::transport::{
        DataLane,
        DefaultWireBuilder,
        DefaultWireBuilderConfig,
        Dest,
        DestTransport,
        InprocTransportManager, // Intra-process transport manager
        NetworkError,
        NetworkResult,
        OutprocTransportManager, // Cross-process transport manager
        WireBuilder,
        WireHandle,
    };

    // Backward compatible alias (deprecated)
    #[allow(deprecated)]
    pub use crate::transport::TransportManager;

    // Utility modules
    pub use crate::error::{ActorResult, ActrError};
    pub use crate::monitoring::{Alert, AlertSeverity, Monitor};
    pub use crate::resource::{ResourceManager, ResourceQuota, ResourceUsage};

    // Base types
    pub use actr_protocol::{ActrId, ActrType};

    // Framework traits (for implementing Workload)
    pub use actr_framework::{Context, Workload};

    // Async trait support
    pub use async_trait::async_trait;

    // Common utilities
    pub use anyhow::{Context as AnyhowContext, Result as AnyhowResult};
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;

    // Tokio runtime primitives
    pub use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
    pub use tokio::time::{Duration, Instant, sleep, timeout};

    // Logging
    pub use tracing::{debug, error, info, trace, warn};
}

pub const INITIAL_CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
