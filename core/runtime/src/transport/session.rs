//! Connection Session
//!
//! Each connection attempt generates a unique session. Even if the same peer
//! reconnects, a new session_id distinguishes old callbacks from new ones.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio_util::sync::CancellationToken;

/// Global session ID generator (monotonically increasing)
static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Connection session — lightweight identity for a single connection attempt
///
/// # Design
/// - `session_id`: Globally unique across all connections (even same peer_id)
/// - `cancel_token`: Cancelled during cleanup to silence stale DC callbacks
/// - `closed` (AtomicBool): Ensures `close()` executes exactly once
///
/// All three fields are `Clone`-shared via Arc/CancellationToken,
/// so cloning a session gives a handle to the same underlying state.
#[derive(Clone, Debug)]
pub struct ConnectionSession {
    /// Globally unique session ID
    pub session_id: u64,
    /// Cancellation token: cancelled during cleanup to silence stale callbacks
    pub cancel_token: CancellationToken,
    /// Close-once flag: ensures close() executes exactly once
    closed: Arc<AtomicBool>,
}

impl ConnectionSession {
    pub fn new() -> Self {
        Self {
            session_id: NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed),
            cancel_token: CancellationToken::new(),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attempt to mark as closed. Returns `true` if this is the first close.
    pub fn try_close(&self) -> bool {
        self.closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
}

impl Default for ConnectionSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_id_monotonic() {
        let s1 = ConnectionSession::new();
        let s2 = ConnectionSession::new();
        assert!(s2.session_id > s1.session_id);
    }

    #[test]
    fn test_try_close_idempotent() {
        let session = ConnectionSession::new();
        assert!(session.try_close());
        assert!(!session.try_close());
        assert!(session.is_closed());
    }

    #[test]
    fn test_cancel_token() {
        let session = ConnectionSession::new();
        assert!(!session.is_cancelled());
        session.cancel();
        assert!(session.is_cancelled());
    }

    #[test]
    fn test_clone_shares_state() {
        let s1 = ConnectionSession::new();
        let s2 = s1.clone();
        s1.cancel();
        assert!(s2.is_cancelled());
        assert!(s1.try_close());
        assert!(!s2.try_close());
    }
}
