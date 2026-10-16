//! Read-only views (the reference's viewers, `Editor.isViewer`): the text
//! takes the focus and shows a caret, but cannot be changed. An attempt to
//! change it (typing, pasting, deleting, undo) shows the reference's
//! information hint "This view is read-only" above the caret
//! (`EditorModificationUtil.checkModificationAllowed`), which goes away with
//! the next key, a caret move or scrolling (`HIDE_BY_ANY_KEY`,
//! `HIDE_BY_SCROLLING`).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Subscription, anchored, deferred, div, point, px,
};
use themes::{Metric, UiRole};
use ui::theme::ThemeExt as _;

use crate::Editor;

/// The hint's text (the reference's `editing.viewer.hint`).
pub const READ_ONLY_HINT: &str = "This view is read-only";

/// The read-only hint while it is shown.
pub(crate) struct ReadOnlyHint {
    /// Hides it at the next key.
    _keys: Subscription,
}

impl Editor {
    /// Makes this editor a read-only view (a preview that shows a file):
    /// changes of the text are refused with the read-only hint.
    pub fn set_viewer(&mut self, viewer: bool) {
        self.viewer = viewer;
    }

    pub fn is_viewer(&self) -> bool {
        self.viewer
    }

    /// The read-only hint is shown.
    pub fn read_only_hint(&self) -> Option<&'static str> {
        self.read_only_hint.as_ref().map(|_| READ_ONLY_HINT)
    }

    /// A change of the text was asked for: refused in a viewer (with the
    /// hint); `true` when it may go ahead.
    pub(crate) fn modification_allowed(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.viewer {
            return true;
        }
        let editor = cx.entity().downgrade();
        let keys = cx.intercept_keystrokes(move |_, _, cx| {
            if let Some(editor) = editor.upgrade() {
                editor.update(cx, |e, cx| e.hide_read_only_hint(cx));
            }
        });
        self.read_only_hint = Some(ReadOnlyHint { _keys: keys });
        cx.notify();
        false
    }

    pub(crate) fn hide_read_only_hint(&mut self, cx: &mut Context<Self>) {
        if self.read_only_hint.take().is_some() {
            cx.notify();
        }
    }
}

/// The hint above the caret, over everything else.
pub(crate) fn render_read_only_hint(
    editor: &Editor,
    cx: &mut Context<Editor>,
) -> Option<AnyElement> {
    editor.read_only_hint.as_ref()?;
    let caret = editor.caret_bounds()?;
    let theme = cx.theme();
    Some(
        deferred(
            anchored()
                .position(point(caret.left(), caret.top() - px(2.0)))
                .anchor(gpui::Corner::BottomLeft)
                .child(
                    div()
                        .id("editor-read-only-hint")
                        .debug_selector(|| "editor-read-only-hint".into())
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(theme.metric(Metric::CornerRadius))
                        .border_1()
                        .border_color(theme.color(UiRole::PopupBorder))
                        .bg(theme.color(UiRole::SurfaceRaised))
                        .text_color(theme.color(UiRole::Text))
                        .text_size(theme.ui_size())
                        .shadow(ui::components::popup_shadow())
                        .whitespace_nowrap()
                        .child(READ_ONLY_HINT),
                ),
        )
        .with_priority(3)
        .into_any_element(),
    )
}
