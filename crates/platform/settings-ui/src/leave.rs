//! Changes not applied when the project closes, the application quits or
//! the Settings window is opened for another project: the reference's
//! "Unsaved Changes" message with Apply, Don't Save and Cancel
//! (`SettingsNonModalDialog.resolveUnsavedChanges`).

use gpui::{AnyWindowHandle, App, PromptLevel};

use crate::window::find;

/// The message's title and buttons (`settings.switch.project.*`).
pub const TITLE: &str = "Unsaved Changes";
pub const APPLY: &str = "Apply";
pub const DONT_SAVE: &str = "Don't Save";
pub const CANCEL: &str = "Cancel";

/// What is about to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaving {
    /// The project of the Settings window closes.
    Project,
    /// The application quits.
    Application,
    /// The window opens for the project with this name.
    Switch(String),
}

/// The message for the project named `project`.
pub fn message(leaving: &Leaving, project: &str) -> String {
    match leaving {
        Leaving::Project => format!(
            "You have unsaved changes in the settings for \"{project}\". Do you want to apply them before closing the project?"
        ),
        Leaving::Application => format!(
            "You have unsaved changes in the settings for \"{project}\". Do you want to apply them before closing the application?"
        ),
        Leaving::Switch(to) => format!(
            "You have unsaved changes in the settings for \"{project}\". Do you want to apply them before switching to \"{to}\"?"
        ),
    }
}

/// Runs `then` once the Settings window's changes are settled: at once
/// without a window or changes; otherwise after the message, when Apply
/// succeeded or the answer was Don't Save (the window is closed then).
/// Cancel, or an Apply that fails, leaves the window open in front and
/// `then` does not run.
pub fn resolve(leaving: Leaving, cx: &mut App, then: impl FnOnce(&mut App) + 'static) {
    let Some(handle) = find(cx) else {
        then(cx);
        return;
    };
    let Ok(view) = handle.read(cx) else {
        then(cx);
        return;
    };
    if !view.any_modified(cx) {
        then(cx);
        return;
    }
    let project = view.project_name.clone().unwrap_or_default();
    let text = message(&leaving, &project);
    let answer = handle.update(cx, |_, window, cx| {
        window.activate_window();
        window.prompt(
            PromptLevel::Warning,
            TITLE,
            Some(&text),
            &[APPLY, DONT_SAVE, CANCEL],
            cx,
        )
    });
    let Ok(answer) = answer else {
        then(cx);
        return;
    };
    cx.spawn(async move |cx| {
        let Ok(choice) = answer.await else {
            return;
        };
        let _ = cx.update(|cx| {
            let go_on = match choice {
                0 => handle
                    .update(cx, |view, window, cx| {
                        let applied = view.apply(window, cx);
                        if applied {
                            view.close(window, cx);
                        }
                        applied
                    })
                    .unwrap_or(true),
                1 => {
                    let _ = handle.update(cx, |view, window, cx| view.close(window, cx));
                    true
                }
                _ => false,
            };
            if go_on {
                then(cx);
            }
        });
    })
    .detach();
}

/// A main window is about to close: true when nothing is to be settled.
/// When the Settings window opened from it has unsaved changes, asks first
/// and runs `then` afterwards if the answer lets the window close.
pub fn settle_parent(
    parent: AnyWindowHandle,
    cx: &mut App,
    then: impl FnOnce(&mut App) + 'static,
) -> bool {
    let Some(handle) = find(cx) else {
        return true;
    };
    if ui::dialog_window::parent_of(handle.into(), cx) != Some(parent) {
        return true;
    }
    if !handle.read(cx).is_ok_and(|v| v.any_modified(cx)) {
        return true;
    }
    resolve(Leaving::Project, cx, then);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_messages_name_the_projects() {
        assert_eq!(
            message(&Leaving::Switch("other".into()), "demo"),
            "You have unsaved changes in the settings for \"demo\". Do you want to apply them before switching to \"other\"?"
        );
        assert!(message(&Leaving::Application, "demo").ends_with("closing the application?"));
    }
}
