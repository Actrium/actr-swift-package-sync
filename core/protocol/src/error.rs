//! Top-level error types for the Actor-RTC framework.
//!
//! ## Design
//!
//! Two layers only:
//!
//! ```text
//! NetworkError   (transport-internal, never exposed to users)
//!      ↓  From
//! ActrError      (public, flat enum — what callers see)
//! ```
//!
//! `RuntimeError` and `ProtocolError` have been removed.
//!
//! ## Error classification
//!
//! Every error belongs to one fault domain (`ErrorKind`):
//!
//! | Kind      | Meaning                        | Retry? | DLQ? |
//! |-----------|--------------------------------|--------|------|
//! | Transient | Environmental fluctuation      | yes    | no   |
//! | Client    | Caller error (bad request)     | no     | no   |
//! | Internal  | Framework bug / panic          | no     | no   |
//! | Corrupt   | Data corruption                | no     | yes  |
//!
//! Use the `Classify` trait to query classification from any error type.

use thiserror::Error;

// ── ActrError ────────────────────────────────────────────────────────────────

/// Top-level framework error, returned to all callers.
///
/// Flat enum — no nested error wrapping. Each variant is self-describing.
#[derive(Error, Debug, Clone)]
pub enum ActrError {
    // ── Transient ──────────────────────────────────────────────────────────
    /// Peer temporarily unavailable: connection lost, overloaded, or reconnecting.
    ///
    /// `ErrorKind::Transient` — retry with backoff.
    #[error("unavailable: {0}")]
    Unavailable(String),

    /// Request deadline exceeded.
    ///
    /// `ErrorKind::Transient` — may retry with a fresh deadline.
    #[error("timed out")]
    TimedOut,

    // ── Client ─────────────────────────────────────────────────────────────
    /// Target actor not found.
    ///
    /// `ErrorKind::Client` — do not retry; check service discovery first.
    #[error("not found: {0}")]
    NotFound(String),

    /// Permission denied by ACL.
    ///
    /// `ErrorKind::Client` — do not retry; fix authorization.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Invalid argument or malformed request.
    ///
    /// `ErrorKind::Client` — do not retry; fix the request.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// No handler registered for the given route key.
    ///
    /// `ErrorKind::Client` — do not retry; check service definition.
    #[error("unknown route: {0}")]
    UnknownRoute(String),

    /// Required dependency not found in the lock file.
    ///
    /// `ErrorKind::Client` — do not retry; fix the manifest.
    #[error("dependency '{service_name}' not found: {message}")]
    DependencyNotFound {
        service_name: String,
        message: String,
    },

    // ── Corrupt ────────────────────────────────────────────────────────────
    /// Protobuf decode failure — message data is corrupted.
    ///
    /// `ErrorKind::Corrupt` — route to Dead Letter Queue; do not retry.
    #[error("decode failure: {0}")]
    DecodeFailure(String),

    // ── Internal ───────────────────────────────────────────────────────────
    /// Feature not yet implemented.
    ///
    /// `ErrorKind::Internal` — do not retry.
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// Internal framework error: bug, panic, or unrecoverable state.
    ///
    /// `ErrorKind::Internal` — do not retry; investigate logs.
    #[error("internal error: {0}")]
    Internal(String),
}

// ── ErrorKind ────────────────────────────────────────────────────────────────

/// Fault domain classification for any framework error.
///
/// All error types implement [`Classify`] to expose their `ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Environmental fluctuation — retry with exponential backoff.
    Transient,
    /// Caller error — bad request or system state; do not retry.
    Client,
    /// Framework bug or panic — do not retry; alert.
    Internal,
    /// Data corruption — route to Dead Letter Queue; manual intervention required.
    Corrupt,
}

// ── Classify trait ───────────────────────────────────────────────────────────

/// Fault-domain classification for error types.
///
/// Implement `kind()` only; `is_retryable()` and `requires_dlq()` have
/// correct default implementations derived from `kind()`.
pub trait Classify {
    /// Returns the fault domain this error belongs to.
    fn kind(&self) -> ErrorKind;

    /// Returns `true` if the operation may be retried.
    ///
    /// Only `ErrorKind::Transient` errors are retryable.
    fn is_retryable(&self) -> bool {
        matches!(self.kind(), ErrorKind::Transient)
    }

    /// Returns `true` if the message should be routed to the Dead Letter Queue.
    ///
    /// Only `ErrorKind::Corrupt` errors require DLQ routing.
    fn requires_dlq(&self) -> bool {
        matches!(self.kind(), ErrorKind::Corrupt)
    }
}

impl Classify for ActrError {
    fn kind(&self) -> ErrorKind {
        match self {
            ActrError::Unavailable(_) | ActrError::TimedOut => ErrorKind::Transient,

            ActrError::NotFound(_)
            | ActrError::PermissionDenied(_)
            | ActrError::InvalidArgument(_)
            | ActrError::UnknownRoute(_)
            | ActrError::DependencyNotFound { .. } => ErrorKind::Client,

            ActrError::DecodeFailure(_) => ErrorKind::Corrupt,

            ActrError::NotImplemented(_) | ActrError::Internal(_) => ErrorKind::Internal,
        }
    }
}

// ── Convenience type aliases ──────────────────────────────────────────────────

/// Result type for actor RPC calls.
pub type ActorResult<T> = Result<T, ActrError>;

#[cfg(test)]
mod tests {
    use super::*;

    // ── ActrError::kind() classification ─────────────────────────────────────

    #[test]
    fn transient_variants_classify_correctly() {
        assert_eq!(
            ActrError::Unavailable("x".into()).kind(),
            ErrorKind::Transient
        );
        assert_eq!(ActrError::TimedOut.kind(), ErrorKind::Transient);
    }

    #[test]
    fn client_variants_classify_correctly() {
        assert_eq!(ActrError::NotFound("x".into()).kind(), ErrorKind::Client);
        assert_eq!(
            ActrError::PermissionDenied("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::InvalidArgument("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::UnknownRoute("x".into()).kind(),
            ErrorKind::Client
        );
        assert_eq!(
            ActrError::DependencyNotFound {
                service_name: "svc".into(),
                message: "not found".into(),
            }
            .kind(),
            ErrorKind::Client
        );
    }

    #[test]
    fn corrupt_variant_classifies_correctly() {
        assert_eq!(
            ActrError::DecodeFailure("x".into()).kind(),
            ErrorKind::Corrupt
        );
    }

    #[test]
    fn internal_variants_classify_correctly() {
        assert_eq!(
            ActrError::NotImplemented("x".into()).kind(),
            ErrorKind::Internal
        );
        assert_eq!(ActrError::Internal("x".into()).kind(), ErrorKind::Internal);
    }

    // ── Classify default impls ────────────────────────────────────────────────

    #[test]
    fn only_transient_is_retryable() {
        assert!(ActrError::Unavailable("x".into()).is_retryable());
        assert!(ActrError::TimedOut.is_retryable());

        assert!(!ActrError::NotFound("x".into()).is_retryable());
        assert!(!ActrError::DecodeFailure("x".into()).is_retryable());
        assert!(!ActrError::Internal("x".into()).is_retryable());
    }

    #[test]
    fn only_corrupt_requires_dlq() {
        assert!(ActrError::DecodeFailure("x".into()).requires_dlq());

        assert!(!ActrError::Unavailable("x".into()).requires_dlq());
        assert!(!ActrError::TimedOut.requires_dlq());
        assert!(!ActrError::NotFound("x".into()).requires_dlq());
        assert!(!ActrError::Internal("x".into()).requires_dlq());
    }

    // ── Clone ─────────────────────────────────────────────────────────────────

    #[test]
    fn actr_error_is_clone() {
        let e = ActrError::InvalidArgument("bad".into());
        let cloned = e.clone();
        assert_eq!(format!("{cloned}"), "invalid argument: bad");
    }
}
