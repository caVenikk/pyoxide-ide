//! Settings of the workbench, in code with their defaults until there is a
//! settings window.

use std::ops::RangeInclusive;
use std::time::Duration;

/// When modified files are written without an explicit save (ADR 0007).
///
/// The reference IDE has two of these settings: saving when its window is
/// deactivated (on by default) and saving after the IDE has been idle for
/// N seconds (off by default, N = 15, 1 to 300), idle meaning no keyboard
/// or mouse input. Pyoxide turns the idle save on with N = 2 and also saves
/// a file when its tab is left; a tab that is closed is always saved, and
/// so are the open files when the application quits. A file that changed
/// or went away on disk is not written over (see `SyncSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveSettings {
    /// Save every modified file when the window loses the focus to another
    /// application.
    pub save_on_deactivation: bool,
    /// Save every modified file after `idle_seconds` without keyboard or
    /// mouse input.
    pub save_on_idle: bool,
    pub idle_seconds: u32,
    /// Save the file of a tab when another tab is selected.
    pub save_on_tab_switch: bool,
}

impl SaveSettings {
    /// The allowed idle times, as in the reference IDE.
    pub const IDLE_SECONDS: RangeInclusive<u32> = 1..=300;

    /// The idle time, clamped to [`Self::IDLE_SECONDS`].
    pub fn idle(&self) -> Duration {
        let s = self
            .idle_seconds
            .clamp(*Self::IDLE_SECONDS.start(), *Self::IDLE_SECONDS.end());
        Duration::from_secs(u64::from(s))
    }
}

impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            save_on_deactivation: true,
            save_on_idle: true,
            idle_seconds: 2,
            save_on_tab_switch: true,
        }
    }
}

/// How changes made to files outside the application are found.
///
/// The reference IDE watches the project with a native file watcher and
/// looks at the files again when its window gets the focus ("Synchronize
/// files on frame activation", on by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncSettings {
    /// Watch the project folder for changes. Off by default so that a
    /// window made by a test sees only the changes it is told about;
    /// [`crate::run`] turns it on.
    pub watch: bool,
    /// Look at the open files (and, without a watcher, the project tree)
    /// when the window gets the focus.
    pub sync_on_activation: bool,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            watch: false,
            sync_on_activation: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_time_is_clamped() {
        let mut s = SaveSettings::default();
        assert_eq!(s.idle(), Duration::from_secs(2));
        s.idle_seconds = 0;
        assert_eq!(s.idle(), Duration::from_secs(1));
        s.idle_seconds = 1000;
        assert_eq!(s.idle(), Duration::from_secs(300));
    }
}
