//! libactr - UniFFI bindings for the Actor-RTC framework
//!
//! This crate provides FFI bindings for the actr framework using Mozilla UniFFI.
//!
//! ## Architecture
//!
//! The actr framework uses complex Rust features (generics, traits, async) that don't map
//! directly to UniFFI. This crate provides a "facade" layer that:
//!
//! 1. Wraps generic types with concrete DynamicWorkload implementation
//! 2. Uses callback interfaces to implement workload logic
//! 3. Exposes simplified APIs for creating and managing actors

mod context;
mod error;
mod logger;
mod opus;
mod runtime;
mod types;
mod workload;

pub use error::*;
pub use logger::*;
pub use opus::*;
pub use runtime::*;
pub use types::*;
pub use workload::*;

// Generate UniFFI scaffolding
uniffi::setup_scaffolding!();
