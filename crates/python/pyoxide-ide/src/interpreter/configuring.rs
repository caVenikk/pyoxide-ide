//! The projects whose interpreter is being configured (the reference
//! IDE's SDK configuration mutex, `observeSdkConfigurationInProgress`):
//! while an interpreter is added for a project, its Interpreter page and
//! the "Add New Interpreter" rows are disabled. Views observe the
//! [`Configuring`] global.

use std::path::{Path, PathBuf};

use gpui::{App, Global};

/// The reference's tooltip of the disabled controls
/// (`active.sdk.dialog.link.add.interpreter.disabled.tooltip`).
pub const BUSY_TOOLTIP: &str = "An interpreter is being configured.";

#[derive(Default)]
pub(crate) struct Configuring(Vec<PathBuf>);

impl Global for Configuring {}

/// An interpreter is being configured for the project at `root`.
pub(crate) fn is_configuring(root: &Path, cx: &App) -> bool {
    cx.try_global::<Configuring>()
        .is_some_and(|c| c.0.iter().any(|r| r == root))
}

/// Starts configuring for `root`; `false` when it is configured already.
pub(crate) fn begin(root: &Path, cx: &mut App) -> bool {
    if is_configuring(root, cx) {
        return false;
    }
    cx.default_global::<Configuring>().0.push(root.to_owned());
    true
}

/// Ends what [`begin`] started.
pub(crate) fn end(root: &Path, cx: &mut App) {
    if is_configuring(root, cx) {
        cx.default_global::<Configuring>().0.retain(|r| r != root);
    }
}
