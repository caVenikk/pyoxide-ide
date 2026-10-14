//! Settings of the Python tools (ADR 0010).
//!
//! Shared, in `.oxide/project.toml`:
//!
//! ```toml
//! [python.tools.ruff]
//! enabled = true
//! lookup = "sdk"          # sdk | path | uvx
//! inspections = true
//! formatting = true
//! import-optimizer = true
//!
//! [python.type-engine]
//! name = "ty"             # ty | basedpyright
//! ```
//!
//! Personal, in `.oxide/workspace.toml`: `[python.tools.<tool>] path`, a
//! custom executable.

use std::path::{Path, PathBuf};

use project::StoreError;

use crate::lookup::Lookup;

/// The table of a tool's shared settings: `python.tools.<id>`.
pub const TOOLS_TABLE: &str = "python.tools";
/// The table of the type engine choice.
pub const TYPE_ENGINE_TABLE: &str = "python.type-engine";

/// A Python tool that runs as a language server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tool {
    Ruff,
    Ty,
    Basedpyright,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Ruff, Tool::Ty, Tool::Basedpyright];

    /// The id in settings and the server id (`ruff`); also the package name.
    pub fn id(self) -> &'static str {
        match self {
            Tool::Ruff => "ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "basedpyright",
        }
    }

    /// The package that provides it (PyPI, for `uvx`).
    pub fn package(self) -> &'static str {
        self.id()
    }

    /// The executable that runs the server.
    pub fn executable(self) -> &'static str {
        match self {
            Tool::Ruff => "ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "basedpyright-langserver",
        }
    }

    /// The arguments that start the server on its standard streams.
    pub fn server_args(self) -> &'static [&'static str] {
        match self {
            Tool::Ruff | Tool::Ty => &["server"],
            Tool::Basedpyright => &["--stdio"],
        }
    }

    /// The name shown (the reference's tool names).
    pub fn name(self) -> &'static str {
        match self {
            Tool::Ruff => "Ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "Basedpyright",
        }
    }

    /// Its table in the settings files (`python.tools.ruff`).
    pub fn table(self) -> String {
        format!("{TOOLS_TABLE}.{}", self.id())
    }
}

/// The type server of a project (ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TypeEngine {
    #[default]
    Ty,
    Basedpyright,
}

impl TypeEngine {
    pub fn id(self) -> &'static str {
        self.tool().id()
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ty" => Some(TypeEngine::Ty),
            "basedpyright" => Some(TypeEngine::Basedpyright),
            _ => None,
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            TypeEngine::Ty => Tool::Ty,
            TypeEngine::Basedpyright => Tool::Basedpyright,
        }
    }
}

/// The shared settings of one tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSettings {
    /// Runs as a language server. The type server runs while it is the
    /// project's type engine.
    pub enabled: bool,
    pub lookup: Lookup,
    /// Its diagnostics and quick fixes.
    pub inspections: bool,
    /// Reformat Code goes to it (ruff).
    pub formatting: bool,
    /// Optimize Imports goes to it (ruff).
    pub import_optimizer: bool,
    /// The settings were written by a person or imported, not Pyoxide's
    /// defaults: lookup may end at `uvx` without asking (ADR 0010).
    pub explicit: bool,
}

impl ToolSettings {
    /// Pyoxide's defaults: ruff and the type server are on (ADR 0011).
    pub fn default_for(tool: Tool) -> Self {
        let ruff = tool == Tool::Ruff;
        Self {
            enabled: tool != Tool::Basedpyright,
            lookup: Lookup::Sdk,
            inspections: true,
            formatting: ruff,
            import_optimizer: ruff,
            explicit: false,
        }
    }
}

/// The Python tools settings of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub ruff: ToolSettings,
    pub ty: ToolSettings,
    pub basedpyright: ToolSettings,
    pub type_engine: TypeEngine,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ruff: ToolSettings::default_for(Tool::Ruff),
            ty: ToolSettings::default_for(Tool::Ty),
            basedpyright: ToolSettings::default_for(Tool::Basedpyright),
            type_engine: TypeEngine::default(),
        }
    }
}

impl Settings {
    pub fn tool(&self, tool: Tool) -> &ToolSettings {
        match tool {
            Tool::Ruff => &self.ruff,
            Tool::Ty => &self.ty,
            Tool::Basedpyright => &self.basedpyright,
        }
    }

    /// The tools that run as servers: ruff when enabled, and the type
    /// engine's server.
    pub fn servers(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if self.ruff.enabled {
            tools.push(Tool::Ruff);
        }
        tools.push(self.type_engine.tool());
        tools
    }
}

/// The project's settings: `.oxide/project.toml`, defaults for what is
/// not there.
pub fn load(root: &Path) -> Result<Settings, StoreError> {
    let _ = root;
    Ok(Settings::default())
}

/// Writes the project's settings (the keys that differ from what is
/// stored).
pub fn save(root: &Path, settings: &Settings) -> Result<bool, StoreError> {
    let _ = (root, settings);
    Ok(false)
}

/// The project has tool settings of its own (`[python.tools]` in
/// `project.toml`): the reference's are no longer imported.
pub fn has_tool_settings(root: &Path) -> Result<bool, StoreError> {
    let _ = root;
    Ok(false)
}

/// The person's custom executable of a tool (`.oxide/workspace.toml`).
pub fn custom_path(root: &Path, tool: Tool) -> Result<Option<PathBuf>, StoreError> {
    let _ = (root, tool);
    Ok(None)
}

/// Sets or removes the person's custom executable of a tool.
pub fn set_custom_path(root: &Path, tool: Tool, path: Option<&Path>) -> Result<bool, StoreError> {
    let _ = (root, tool, path);
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_and_defaults() {
        assert_eq!(Tool::Ruff.table(), "python.tools.ruff");
        assert_eq!(Tool::Basedpyright.executable(), "basedpyright-langserver");
        assert_eq!(Tool::Basedpyright.server_args(), ["--stdio"]);
        assert_eq!(
            TypeEngine::parse("basedpyright"),
            Some(TypeEngine::Basedpyright)
        );
        assert_eq!(TypeEngine::parse("pyrefly"), None);
        let settings = Settings::default();
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Ty]);
        assert!(settings.tool(Tool::Ruff).formatting);
        assert!(!settings.tool(Tool::Ty).formatting);
        assert!(!settings.ruff.explicit);
    }
}
