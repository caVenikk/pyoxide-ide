//! How breakpoints are shown, as the reference computes it: the icon by
//! state (`XBreakpointUIUtil.calculateIcon`), the question badge of a
//! condition, the short text (`main.py:7`) and the tooltip
//! (`XBreakpointBase.getDescription`).

use std::path::Path;

use debugger::{BreakpointStatus, ExceptionBreakpoint, LineBreakpoint, SuspendPolicy};
use ui::icons::IconName;

use crate::icons;

/// The icon of a line breakpoint: disabled (muted or not), muted, what the
/// running debugger says of it (verified or invalid), non-suspending,
/// else the type's icon (`calculateSpecialIcon`).
///
/// The reference shows muted icons while the current session is muted;
/// the mute state here is the project's, so it shows them while muted.
pub fn line_icon(b: &LineBreakpoint, muted: bool, status: Option<&BreakpointStatus>) -> IconName {
    if !b.enabled {
        return if muted {
            icons::MUTED_DISABLED
        } else {
            icons::DISABLED
        };
    }
    if muted {
        return icons::MUTED;
    }
    let suspends = b.suspend != SuspendPolicy::None;
    match status {
        Some(s) if s.verified => {
            if suspends {
                icons::VALID
            } else {
                icons::UNSUSPENDENT_VALID
            }
        }
        // Not verified, with the adapter's reason: it cannot be set.
        Some(s) if s.message.is_some() => icons::INVALID,
        _ if !suspends => icons::UNSUSPENDENT,
        _ => icons::BREAKPOINT,
    }
}

/// The question badge of a breakpoint with a condition
/// (`withQuestionBadgeIfNeeded`).
pub fn badge(condition: Option<&str>) -> Option<IconName> {
    condition
        .is_some_and(|c| !c.trim().is_empty())
        .then_some(icons::QUESTION_BADGE)
}

/// The icon of an exception breakpoint (the type's enabled and disabled
/// icons, the platform's muted ones).
pub fn exception_icon(b: &ExceptionBreakpoint, muted: bool) -> IconName {
    match (b.enabled, muted) {
        (true, false) => icons::EXCEPTION,
        (false, false) => icons::EXCEPTION_DISABLED,
        (true, true) => icons::MUTED,
        (false, true) => icons::MUTED_DISABLED,
    }
}

/// A line breakpoint's text: the file's name and the line
/// (`xbreakpoint.default.display.text`, `main.py:7`).
pub fn short_text(file: &Path, line: u32) -> String {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{name}:{line}")
}

/// The lines of a line breakpoint's tooltip: its text, the debugger's
/// error, the suspend policy when it is none or not the type's default,
/// the condition, what it logs; with nothing of that, "Right-click to
/// edit" (`getDefaultDescription`).
pub fn tooltip(
    b: &LineBreakpoint,
    status: Option<&BreakpointStatus>,
    default_suspend: SuspendPolicy,
    suspend_thread: bool,
) -> Vec<String> {
    let mut lines = vec![short_text(&b.file, b.line)];
    if let Some(message) = status
        .filter(|s| !s.verified)
        .and_then(|s| s.message.clone())
    {
        lines.push(message);
    }
    match b.suspend {
        SuspendPolicy::None => lines.push("Suspend: none".to_owned()),
        policy if suspend_thread && policy != default_suspend => lines.push(
            match policy {
                SuspendPolicy::All => "Suspend: all",
                _ => "Suspend: thread",
            }
            .to_owned(),
        ),
        _ => {}
    }
    if let Some(condition) = b.condition.as_deref().filter(|c| !c.trim().is_empty()) {
        lines.push(format!("Condition: {condition}"));
    }
    if b.log_message {
        lines.push("Log message: yes".to_owned());
    }
    if b.log_stack {
        lines.push("Log stack: yes".to_owned());
    }
    if let Some(expression) = b.log_expression.as_deref().filter(|e| !e.trim().is_empty()) {
        lines.push(format!("Log expression: {expression}"));
    }
    if lines.len() == 1 {
        lines.push(EDIT_HINT.to_owned());
    }
    lines
}

/// The tooltip's hint (`xbreakpoint.tooltip.edit.hint`).
pub const EDIT_HINT: &str = "Right-click to edit";
/// The hover icon's tooltip (`xbreakpoint.add.hover.tooltip`).
pub const ADD_HINT: &str = "Add Breakpoint";

#[cfg(test)]
mod tests {
    use super::*;

    fn status(verified: bool, message: Option<&str>) -> BreakpointStatus {
        BreakpointStatus {
            verified,
            message: message.map(str::to_owned),
            line: None,
        }
    }

    #[test]
    fn icons_by_state() {
        let mut b = LineBreakpoint::new("/p/main.py", 7);
        b.suspend = SuspendPolicy::Thread;
        assert_eq!(line_icon(&b, false, None), icons::BREAKPOINT);
        assert_eq!(line_icon(&b, true, None), icons::MUTED);
        assert_eq!(
            line_icon(&b, false, Some(&status(true, None))),
            icons::VALID
        );
        assert_eq!(
            line_icon(&b, false, Some(&status(false, Some("no code")))),
            icons::INVALID
        );
        // Pending: the plain icon.
        assert_eq!(
            line_icon(&b, false, Some(&status(false, None))),
            icons::BREAKPOINT
        );
        b.suspend = SuspendPolicy::None;
        assert_eq!(line_icon(&b, false, None), icons::UNSUSPENDENT);
        assert_eq!(
            line_icon(&b, false, Some(&status(true, None))),
            icons::UNSUSPENDENT_VALID
        );
        b.enabled = false;
        assert_eq!(line_icon(&b, false, None), icons::DISABLED);
        assert_eq!(line_icon(&b, true, None), icons::MUTED_DISABLED);
        assert_eq!(badge(Some("i == 3")), Some(icons::QUESTION_BADGE));
        assert_eq!(badge(Some("  ")), None);
        assert_eq!(badge(None), None);
        let mut any = ExceptionBreakpoint::new("python-exception", "BaseException");
        assert_eq!(exception_icon(&any, false), icons::EXCEPTION);
        any.enabled = false;
        assert_eq!(exception_icon(&any, false), icons::EXCEPTION_DISABLED);
    }

    #[test]
    fn tooltips_as_the_reference_describes_breakpoints() {
        let mut b = LineBreakpoint::new("/p/main.py", 7);
        b.suspend = SuspendPolicy::Thread;
        let thread = SuspendPolicy::Thread;
        assert_eq!(
            tooltip(&b, None, thread, true),
            ["main.py:7", "Right-click to edit"]
        );
        b.suspend = SuspendPolicy::All;
        b.condition = Some("i == 3".into());
        assert_eq!(
            tooltip(&b, None, thread, true),
            ["main.py:7", "Suspend: all", "Condition: i == 3"]
        );
        b.suspend = SuspendPolicy::None;
        b.condition = None;
        b.log_message = true;
        assert_eq!(
            tooltip(&b, Some(&status(false, Some("No code"))), thread, true),
            ["main.py:7", "No code", "Suspend: none", "Log message: yes"]
        );
    }
}
