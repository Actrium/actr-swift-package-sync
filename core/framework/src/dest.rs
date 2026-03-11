//! Destination identifier for Actor communication
//!
//! Defines the `Dest` enum with three-way destination distinction:
//! - **Shell**: Workload → App (inproc reverse channel)
//! - **Local**: Target local Workload (inproc from App, outproc short-circuit from Workload)
//! - **Actor**: Remote Actor (full outproc)
//!
//! # Design Rationale
//!
//! The three-way distinction provides:
//! - **Clear semantics**: Shell/Local/Actor have distinct meanings
//! - **Symmetric communication**: App ↔ Workload bidirectional calls
//! - **Protocol consistency**: Workload self-calls use full serialization (same as remote)
//! - **Transparent optimization**: Transport layer can short-circuit Local calls
//!
//! # Usage
//!
//! **Shell 侧 (App)**:
//! ```rust,ignore
//! // Call local Workload (隐含 Dest::Local)
//! running_node.call(request).await?;  // No target parameter
//! ```
//!
//! **Actr 侧 (Workload)**:
//! ```rust,ignore
//! // Call App
//! ctx.call(&Dest::Shell, request).await?;
//!
//! // Call self (outproc short-circuit)
//! ctx.call(&Dest::Local, request).await?;
//!
//! // Call remote Actor
//! ctx.call(&Dest::Actor(server_id), request).await?;
//! ```
//!
//! # Placement in actr-framework
//!
//! `Dest` is placed in `actr-framework` (not `actr-protocol`) because:
//! - It's an API-level abstraction, not a protocol data type
//! - It's used directly by the `Context` trait
//! - `RpcEnvelope` (in protocol layer) does not contain destination information
//! - The runtime layer implements the routing logic based on `Dest`

use actr_protocol::ActrId;
use std::hash::{Hash, Hasher};

/// Destination identifier
///
/// Three-way destination for message routing.
///
/// # Semantics
///
/// - **`Dest::Shell`**: Workload → App (inproc 反向通道)
///   - Used by Workload to call App side
///   - Routed through `InprocOutGate` (zero serialization)
///   - Example: Workload pushing notifications to App
///
/// - **`Dest::Local`**: Target local Workload
///   - From App: routed through `InprocOutGate` (zero serialization)
///   - From Workload: routed through `OutprocOutGate` (full serialization, short-circuit at transport)
///   - Example: App calling its local Workload, or Workload calling itself
///
/// - **`Dest::Actor(ActrId)`**: Remote Actor (full outproc)
///   - Used for cross-process Actor communication
///   - Routed through `OutprocOutGate` (WebRTC/WebSocket)
///   - Example: ClientWorkload calling RemoteServer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dest {
    /// Local Shell - Workload 调用 App 侧 (inproc 反向通道)
    Shell,

    /// Local Workload - 调用本地 Workload (从 App: inproc, 从 Workload: outproc 短接)
    Local,

    /// Remote Actor - 跨进程通信 (WebRTC/WebSocket)
    Actor(ActrId),
}

impl Hash for Dest {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Dest::Shell => {
                0u8.hash(state);
            }
            Dest::Local => {
                1u8.hash(state);
            }
            Dest::Actor(id) => {
                2u8.hash(state);
                id.hash(state);
            }
        }
    }
}

impl Dest {
    /// Create Shell destination
    #[inline]
    pub fn shell() -> Self {
        Dest::Shell
    }

    /// Create Local destination
    #[inline]
    pub fn local() -> Self {
        Dest::Local
    }

    /// Create Actor destination
    #[inline]
    pub fn actor(id: ActrId) -> Self {
        Dest::Actor(id)
    }

    /// Check if this is a Shell destination
    #[inline]
    pub fn is_shell(&self) -> bool {
        matches!(self, Dest::Shell)
    }

    /// Check if this is a Local destination
    #[inline]
    pub fn is_local(&self) -> bool {
        matches!(self, Dest::Local)
    }

    /// Check if this is an Actor destination
    #[inline]
    pub fn is_actor(&self) -> bool {
        matches!(self, Dest::Actor(_))
    }

    /// Get ActrId (if this is an Actor destination)
    ///
    /// Returns `None` for `Dest::Shell` or `Dest::Local`.
    #[inline]
    pub fn as_actor_id(&self) -> Option<&ActrId> {
        match self {
            Dest::Actor(id) => Some(id),
            _ => None,
        }
    }
}

impl From<ActrId> for Dest {
    #[inline]
    fn from(id: ActrId) -> Self {
        Dest::Actor(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dest_creation() {
        let shell_dest = Dest::shell();
        assert!(shell_dest.is_shell());
        assert!(!shell_dest.is_local());
        assert!(!shell_dest.is_actor());

        let local_dest = Dest::local();
        assert!(!local_dest.is_shell());
        assert!(local_dest.is_local());
        assert!(!local_dest.is_actor());

        let id = ActrId::default();
        let actor_dest = Dest::actor(id);
        assert!(!actor_dest.is_shell());
        assert!(!actor_dest.is_local());
        assert!(actor_dest.is_actor());
    }

    #[test]
    fn test_dest_hash() {
        use std::collections::HashMap;

        let id1 = ActrId::default();
        let mut id2 = ActrId::default();
        id2.serial_number = 1; // Ensure different ID

        let mut map = HashMap::new();
        map.insert(Dest::shell(), "shell");
        map.insert(Dest::local(), "local");
        map.insert(Dest::actor(id1), "actor1");
        map.insert(Dest::actor(id2), "actor2");

        assert_eq!(map.len(), 4);
    }

    #[test]
    fn test_dest_as_actor_id() {
        let shell_dest = Dest::shell();
        assert_eq!(shell_dest.as_actor_id(), None);

        let local_dest = Dest::local();
        assert_eq!(local_dest.as_actor_id(), None);

        let id = ActrId::default();
        let actor_dest = Dest::actor(id.clone());
        assert_eq!(actor_dest.as_actor_id(), Some(&id));
    }
}
