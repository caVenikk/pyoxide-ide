//! Cooperative cancellation of searches that run on another thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A flag shared between whoever started a search and the search itself;
/// the search checks it between matches and stops once it is set.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks every search holding this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// A search stopped because its token was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the search was cancelled")
    }
}

impl std::error::Error for Cancelled {}
