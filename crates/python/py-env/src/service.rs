use std::sync::Arc;

use crate::discovery::{SearchEnv, Tools};
use crate::error::{EnvError, Result};
use crate::search::{PackageIndex, PyPi};
use crate::uv::{Uv, UvConfig};

/// What the `env.*` operations run on: the uv installation (if any), the
/// other tools found at start-up, and the package index. Build one per
/// application; tests pass a fake index and a fake `uv`.
#[derive(Clone)]
pub struct EnvService {
    uv: Option<Uv>,
    tools: Tools,
    search_env: SearchEnv,
    index: Arc<dyn PackageIndex>,
}

impl EnvService {
    /// Locate tools in the process environment and use PyPI.
    pub fn from_process() -> Self {
        let tools = Tools::locate();
        let uv = tools.uv.as_ref().map(|p| Uv::new(p, UvConfig::default()));
        Self {
            uv,
            tools,
            search_env: SearchEnv::from_process(),
            index: Arc::new(PyPi::new()),
        }
    }

    /// Fully explicit, for tests and embedders.
    pub fn new(
        uv: Option<Uv>,
        tools: Tools,
        search_env: SearchEnv,
        index: Arc<dyn PackageIndex>,
    ) -> Self {
        let mut tools = tools;
        tools.uv = uv.as_ref().map(|u| u.program().to_path_buf());
        Self {
            uv,
            tools,
            search_env,
            index,
        }
    }

    pub fn with_index(mut self, index: Arc<dyn PackageIndex>) -> Self {
        self.index = index;
        self
    }

    /// Replace uv (or remove it, to exercise the pip fallback).
    pub fn with_uv(mut self, uv: Option<Uv>) -> Self {
        self.tools.uv = uv.as_ref().map(|u| u.program().to_path_buf());
        self.uv = uv;
        self
    }

    pub fn uv(&self) -> Option<&Uv> {
        self.uv.as_ref()
    }

    /// uv, or a [`ToolMissing`](crate::ErrorKind::ToolMissing) error that says what needs it.
    pub fn require_uv(&self, what: &str) -> Result<&Uv> {
        self.uv.as_ref().ok_or_else(|| {
            EnvError::tool_missing(
                "uv",
                &format!(
                    "{what} needs uv. Install it from https://docs.astral.sh/uv/ and try again."
                ),
            )
        })
    }

    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    pub fn search_env(&self) -> &SearchEnv {
        &self.search_env
    }

    pub fn index(&self) -> &Arc<dyn PackageIndex> {
        &self.index
    }
}

impl std::fmt::Debug for EnvService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvService")
            .field("uv", &self.uv.as_ref().map(Uv::program))
            .field("tools", &self.tools)
            .finish()
    }
}
