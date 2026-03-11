#![allow(deprecated)]
//! # Actor-RTC Protocol, Types, and URI Parsing
//!
//! This crate contains the unified protocol definitions for the Actor-RTC framework.
//! Its primary role is to provide the raw, generated Protobuf types and essential,
//! stateless utilities for handling those types (e.g., ID and URI formatting).
//!
//! It strictly adheres to its role as a data definition layer and does not contain
//! higher-level traits, business logic, or runtime-specific implementations.

// Include generated protobuf code from prost
pub mod generated {
    pub mod actr {
        include!(concat!(env!("OUT_DIR"), "/actr.rs"));
    }
    pub mod signaling {
        include!(concat!(env!("OUT_DIR"), "/signaling.rs"));
    }
    pub mod webrtc {
        include!(concat!(env!("OUT_DIR"), "/webrtc.rs"));
    }
}

// Re-export all generated types at the crate root for convenience
pub use generated::actr::*;
pub use generated::signaling::*;
pub use generated::webrtc::*;

// Stateless, self-contained extensions and utilities
pub mod actr_ext;
pub mod error;
pub mod message;
pub mod name;
pub mod turn;
pub mod uri;

// Re-export key utilities for convenience
pub use actr_ext::*;
pub use error::*;
pub use message::RpcRequest;
pub use name::*;

// Re-export prost and prost_types for downstream crates
// This ensures a single source of truth for protobuf dependencies
pub use prost;
pub use prost_types;

#[cfg(feature = "uniffi")]
uniffi::setup_scaffolding!();
