//! What a product supplies to the debugger UI: its debug adapter provider
//! (the adapter, the launch, the breakpoint types and stoppable lines).

use std::sync::Arc;

use debugger::DebugAdapterProvider;

/// The product's [`DebugAdapterProvider`] in a `workbench::Product`.
#[derive(Clone)]
pub struct DebugHook(pub Arc<dyn DebugAdapterProvider>);

impl DebugHook {
    pub fn new(provider: impl DebugAdapterProvider) -> Self {
        Self(Arc::new(provider))
    }
}

impl std::fmt::Debug for DebugHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DebugHook")
    }
}
