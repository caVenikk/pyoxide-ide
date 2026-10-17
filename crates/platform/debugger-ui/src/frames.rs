//! The frames view of a debug session (the reference's `XFramesView` in
//! Threads & Variables): the thread combo (the selected thread with its
//! icon; a click lists the threads), the selected thread's frames
//! (`describe, main.py:7:1`, frames of files outside the project on the
//! non-project background, `XDebuggerFramesList`) and the dismissable hint
//! "Switch frames from anywhere in the IDE with ⌥⌘↑ and ⌥⌘↓"
//! (`XFramesView.addFramesNavigationAd`).

use std::path::Path;

use debugger::{FramePresentation, Phase, SessionSnapshot, StackFrame};
use gpui::{
    AnyElement, App, ClickEvent, Context, MouseButton, MouseDownEvent, SharedString, Window, div,
    prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::icon_button;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::icons;
use crate::session::commands::{CloseFramesHint, NextFrame, PreviousFrame, SelectFrame, ShowThreads};
use crate::session::tool_window::DebugToolWindow;

/// The key context of the frames list (its keys: `workbench/keymaps`).
pub const KEY_CONTEXT: &str = "DebugFrames";
/// The thread combo's height.
const COMBO: f32 = 30.0;
/// The reference's empty text (`XDebuggerBundle`
/// `debugger.frames.not.available`).
const FRAMES_NOT_AVAILABLE: &str = "Frames are not available";

/// A frame as the frames list shows it: `describe, main.py:7:1` (the
/// line and the column), or its name alone without a file.
pub fn frame_text(frame: &StackFrame) -> String {
    let file = frame
        .source
        .as_ref()
        .and_then(|s| {
            s.path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .or_else(|| s.name.clone())
        })
        .unwrap_or_default();
    if file.is_empty() {
        frame.name.clone()
    } else {
        format!("{}, {file}:{}:{}", frame.name, frame.line, frame.column.max(1))
    }
}

/// A frame of code outside the project: the adapter shows it less
/// prominently (library code), or its file is not under the project root.
pub fn is_library(frame: &StackFrame, root: Option<&Path>) -> bool {
    if frame.presentation == FramePresentation::Subtle
        || frame.source.as_ref().is_some_and(|s| s.deemphasized)
    {
        return true;
    }
    match (frame.source.as_ref().and_then(|s| s.path.as_ref()), root) {
        (Some(path), Some(root)) => !path.starts_with(root),
        _ => false,
    }
}

/// The index of the selected frame among the frames.
pub(crate) fn selected_index(snapshot: &SessionSnapshot) -> Option<usize> {
    let selected = snapshot.selected_frame?;
    snapshot.frames.iter().position(|f| f.id == selected)
}

impl DebugToolWindow {
    /// The frames pane of a tab: the thread combo, the frames, the hint.
    pub(crate) fn render_frames(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab = &self.tabs[index];
        let s = tab.shown_frames();
        let theme = cx.theme();
        let focused = self.frames_focus.contains_focused(window, cx);
        let thread = (s.phase == Phase::Suspended)
            .then(|| {
                s.threads
                    .iter()
                    .find(|t| Some(t.id) == s.selected_thread)
                    .map(|t| t.name.clone())
            })
            .flatten();
        let combo = div()
            .id("debug-threads")
            .debug_selector(|| "debug-threads".into())
            .flex_none()
            .h(px(COMBO))
            .pl(px(8.0))
            .pr(px(6.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Border))
            .when_some(thread, |d, name| {
                d.child(Icon::new(icons::THREAD_CURRENT).size(px(16.0)))
                    .child(div().flex_1().whitespace_nowrap().child(name))
                    .child(Icon::new(IconName::CHEVRON_DOWN).size(px(16.0)))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(ShowThreads), cx))
            });
        let selected = selected_index(&s);
        let root = self.root.clone();
        let selection = if focused {
            theme.color(UiRole::Selection)
        } else {
            theme.color(UiRole::SelectionInactive)
        };
        let library_bg = theme.color(UiRole::NonProjectFile);
        let row_h = theme.metric(Metric::RowHeight);
        let radius = theme.metric(Metric::CornerRadius);
        let list: AnyElement = if s.phase == Phase::Suspended && !s.frames.is_empty() {
            div()
                .id("debug-frames")
                .debug_selector(|| "debug-frames".into())
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .py(px(2.0))
                .overflow_y_scroll()
                .children(s.frames.iter().enumerate().map(|(i, f)| {
                    let library = is_library(f, root.as_deref());
                    div()
                        .id(("debug-frame", i))
                        .debug_selector(move || format!("debug-frame-{i}"))
                        .flex_none()
                        .h(row_h)
                        .w_full()
                        .when(library, |d| d.bg(library_bg))
                        .px(px(4.0))
                        .child(
                            div()
                                .size_full()
                                .px(px(4.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(4.0))
                                .rounded(radius)
                                .when(selected == Some(i), |d| d.bg(selection))
                                .child(Icon::new(icons::FRAME).size(px(16.0)))
                                .child(div().whitespace_nowrap().child(frame_text(f))),
                        )
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                window.focus(&this.frames_focus);
                                window.dispatch_action(Box::new(SelectFrame { index: i }), cx);
                            }),
                        )
                }))
                .into_any_element()
        } else {
            let text = if s.phase == Phase::Suspended && s.frames.is_empty() {
                ""
            } else {
                FRAMES_NOT_AVAILABLE
            };
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(text)
                .into_any_element()
        };
        let hint = (!self.frames_hint_closed).then(|| {
            let previous = ui::keys::shortcut_in(&PreviousFrame, &self.frames_focus, window);
            let next = ui::keys::shortcut_in(&NextFrame, &self.frames_focus, window);
            (previous, next)
        });
        let hint = match hint {
            Some((Some(previous), Some(next))) => Some(
                div()
                    .id("debug-frames-hint")
                    .debug_selector(|| "debug-frames-hint".into())
                    .flex_none()
                    .flex()
                    .flex_row()
                    .items_center()
                    .pt(px(3.0))
                    .pb(px(3.0))
                    .pl(px(8.0))
                    .pr(px(4.0))
                    .gap(px(4.0))
                    .text_size(theme.ui_size() - px(1.0))
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(SharedString::from(format!(
                                "Switch frames from anywhere in the IDE with {previous} and {next}"
                            ))),
                    )
                    .child(
                        icon_button(
                            "debug-frames-hint-close",
                            IconName::CLOSE_SMALL,
                            px(16.0),
                            px(16.0),
                            false,
                            cx,
                        )
                        .on_click(|_: &ClickEvent, window, cx| {
                            window.dispatch_action(Box::new(CloseFramesHint), cx)
                        }),
                    ),
            ),
            _ => None,
        };
        div()
            .id("debug-frames-pane")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.frames_focus)
            .flex()
            .flex_col()
            .size_full()
            .child(combo)
            .child(list)
            .children(hint)
            .into_any_element()
    }

    /// The frames pane as `debug.state` reports it.
    pub(crate) fn frames_json(&self, index: usize) -> Json {
        let tab = &self.tabs[index];
        let s = tab.shown_frames();
        let root = self.root.as_deref();
        json!({
            "thread": s.threads.iter().find(|t| Some(t.id) == s.selected_thread).map(|t| t.name.clone()),
            "threads": s.threads.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            "selected": selected_index(&s),
            "rows": s.frames.iter().map(|f| json!({
                "text": frame_text(f),
                "library": is_library(f, root),
            })).collect::<Vec<_>>(),
            "hint": !self.frames_hint_closed,
        })
    }
}

/// Thread and frame keys of the window (the session commands' side).
pub(crate) fn frame_in_direction(snapshot: &SessionSnapshot, delta: isize) -> Option<usize> {
    if snapshot.frames.is_empty() {
        return None;
    }
    let current = selected_index(snapshot).unwrap_or(0) as isize;
    let next = (current + delta).clamp(0, snapshot.frames.len() as isize - 1) as usize;
    (next as isize != current).then_some(next)
}

#[allow(dead_code)]
fn unused(_: &App) {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use debugger::Source;

    use super::*;

    fn frame(name: &str, path: &str, line: u32) -> StackFrame {
        StackFrame {
            id: 1,
            name: name.to_owned(),
            source: Some(Source {
                path: Some(PathBuf::from(path)),
                ..Source::default()
            }),
            line,
            column: 1,
            presentation: FramePresentation::Normal,
        }
    }

    #[test]
    fn frames_show_the_line_and_the_column() {
        assert_eq!(
            frame_text(&frame("describe", "/p/main.py", 7)),
            "describe, main.py:7:1"
        );
        let mut f = frame("<module>", "/p/main.py", 19);
        f.source = None;
        assert_eq!(frame_text(&f), "<module>");
    }

    #[test]
    fn library_frames_are_outside_the_project_or_subtle() {
        let root = Path::new("/p");
        assert!(!is_library(&frame("f", "/p/main.py", 1), Some(root)));
        assert!(is_library(&frame("f", "/usr/lib/runpy.py", 1), Some(root)));
        let mut f = frame("f", "/p/main.py", 1);
        f.presentation = FramePresentation::Subtle;
        assert!(is_library(&f, Some(root)));
    }
}
