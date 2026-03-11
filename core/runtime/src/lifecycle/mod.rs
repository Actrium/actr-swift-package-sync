//! Lifecycle management layer (non-architectural layer)
//!
//! Responsible for Actor system lifecycle management:
//! - ActrSystem: Initialization and configuration
//! - ActrNode<W>: Generic node (bound to Workload Type)

mod actr_node;
mod actr_system;
pub mod compat_lock;
pub mod dedup;
mod heartbeat;
mod network_event;

pub use actr_node::{ActrNode, CredentialState, DiscoveryResult};
pub use actr_system::ActrSystem;
pub use compat_lock::{CompatLockFile, CompatLockManager, CompatibilityCheck, NegotiationEntry};
pub use heartbeat::heartbeat_task;
pub use network_event::{
    DebounceConfig, DefaultNetworkEventProcessor, NetworkEvent, NetworkEventHandle,
    NetworkEventProcessor, NetworkEventResult,
};
