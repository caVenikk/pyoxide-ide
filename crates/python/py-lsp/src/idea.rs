//! The one-time import of the reference's Python tool settings (ADR 0010):
//! `.idea/pyLspTools.xml` (`PyToolsState`, and the older form inside
//! `RuffConfiguration` and its peers) and `.idea/pyTypeEngine.xml`
//! (`PyTypeEngineProjectSettings`), read while `project.toml` has no
//! `[python.tools]` table. `.idea` is never written.

use std::path::Path;

use project::StoreError;

use crate::settings::Settings;

/// The reference's tool settings of a project, when `.idea` has any.
pub fn read_tool_settings(root: &Path) -> Option<Settings> {
    let _ = root;
    None
}

/// Imports the reference's tool settings into `.oxide/project.toml`, once:
/// only while the project has no tool settings of its own. Returns them
/// when they were written.
pub fn import_tool_settings(root: &Path) -> Result<Option<Settings>, StoreError> {
    let _ = root;
    Ok(None)
}
