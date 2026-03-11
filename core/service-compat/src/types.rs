//! Core types for actr-version compatibility analysis
//!
//! These are business logic types specific to compatibility analysis,
//! separate from the protocol layer types in actr-protocol.

use serde::{Deserialize, Serialize};

/// Compatibility level between two protocol versions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum CompatibilityLevel {
    /// No changes detected (semantic fingerprints match)
    FullyCompatible = 0,
    /// Changes present but backward compatible
    BackwardCompatible = 1,
    /// Breaking changes detected
    BreakingChanges = 2,
}

/// Simplified proto file representation for convenience
/// (adapts from actr_protocol::service_spec::Protobuf)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtoFile {
    /// File name (e.g., "user.proto")
    pub name: String,
    /// Proto file content
    pub content: String,
    /// Optional file path (for local development)
    pub path: Option<String>,
}

impl ProtoFile {
    /// Extract filename from URI or path
    /// e.g., "actr://service/proto/user.proto" → "user.proto"
    /// Used for compatibility with old URI-based references
    pub fn name_from_uri(uri: &str) -> String {
        uri.rsplit('/').next().unwrap_or(uri).to_string()
    }

    /// Create from protocol Protobuf message (package-level)
    /// The Protobuf structure represents a merged package, so we use package name directly
    pub fn from_protobuf(proto: &actr_protocol::service_spec::Protobuf) -> Self {
        Self {
            name: proto.package.clone(), // Package name (e.g., "user.v1")
            content: proto.content.clone(),
            path: None,
        }
    }
}
