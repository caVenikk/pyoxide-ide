//! What a product supplies to the debugger UI: its debug adapter provider
//! (the adapter, the launch, the breakpoint types and stoppable lines) and
//! the step that finds what a debug run needs on the UI thread (the
//! interpreter the product picks for it).

use std::rc::Rc;
use std::sync::Arc;

use debugger::{DebugAdapterProvider, DebugRequest};
use gpui::{App, Task};

/// The product's step before a session starts, on the UI thread: it may
/// look things up in the background and returns the request with
/// [`DebugRequest::resolved`] filled in, or a message for the user.
pub type Resolve = Rc<dyn Fn(DebugRequest, &mut App) -> Task<Result<DebugRequest, String>>>;

/// The product's [`DebugAdapterProvider`] in a `workbench::Product`.
#[derive(Clone)]
pub struct DebugHook {
    pub provider: Arc<dyn DebugAdapterProvider>,
    pub resolve: Option<Resolve>,
}

impl DebugHook {
    pub fn new(provider: impl DebugAdapterProvider) -> Self {
        Self::shared(Arc::new(provider))
    }

    /// A hook with a provider shared with others.
    pub fn shared(provider: Arc<dyn DebugAdapterProvider>) -> Self {
        Self {
            provider,
            resolve: None,
        }
    }

    /// The product's step before a session starts.
    pub fn with_resolve(
        mut self,
        resolve: impl Fn(DebugRequest, &mut App) -> Task<Result<DebugRequest, String>> + 'static,
    ) -> Self {
        self.resolve = Some(Rc::new(resolve));
        self
    }
}

impl std::fmt::Debug for DebugHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DebugHook")
    }
}
