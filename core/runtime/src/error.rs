//! Runtime error re-exports.
//!
//! `RuntimeError` has been removed. All errors flow as `ActrError` (public)
//! or `NetworkError` (transport-internal). See `actr_protocol::error` for the
//! full error design.

pub use actr_protocol::{ActorResult, ActrError, Classify, ErrorKind};
