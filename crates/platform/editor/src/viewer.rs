//! Read-only views (the reference's viewers, `Editor.isViewer`): the text
//! takes the focus and shows a caret, but cannot be changed. An attempt to
//! change it (typing, pasting, deleting, undo) shows the reference's
//! information hint "This view is read-only", a balloon that points at the
//! caret from above its line (`EditorModificationUtil.checkModificationAllowed`),
//! which goes away with the next key, a caret move or scrolling
//! (`HIDE_BY_ANY_KEY`, `HIDE_BY_SCROLLING`).

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder,
    Styled as _, Subscription, anchored, canvas, div, point, px,
};
use themes::UiRole;
use ui::deferred;
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

    /// Hides the line numbers and the marks column (a view of one value, as
    /// the database grid's value panel); the colours stay the editor's.
    pub fn set_bare(&mut self, bare: bool) {
        self.bare = bare;
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

/// Room inside the hint's box: top, left, bottom, right (the reference's
/// information hint in the New UI, `HintHint.applyStatus`).
const HINT_PADDING: [f32; 4] = [12.0, 12.0, 14.0, 12.0];
/// Rounding of the box (a balloon with its pointer at the start,
/// `IdeTooltipManager`).
const HINT_RADIUS: f32 = 8.0;
/// The pointer: width and height (`LightweightHint.showAwtTooltip`), and
/// the distance of its left end from the box's left edge (shifted to the
/// start).
const POINTER: (f32, f32) = (16.0, 8.0);
const POINTER_LEFT: f32 = 12.0;
/// Line height of the hint's text.
const HINT_LINE: f32 = 16.0;

/// The hint over everything else: a balloon above the caret's line whose
/// pointer's tip is at the caret, a tenth of a line above the line
/// (`HintManager.showInformationHint` ABOVE, `setCalloutShift`); under the
/// line, pointing up, when there is no room above it.
pub(crate) fn render_read_only_hint(
    editor: &Editor,
    cx: &mut Context<Editor>,
) -> Option<AnyElement> {
    editor.read_only_hint.as_ref()?;
    let caret = editor.caret_bounds()?;
    let theme = cx.theme();
    let callout = caret.size.height * 0.1;
    let (pointer_w, pointer_h) = (px(POINTER.0), px(POINTER.1));
    let [top, left, bottom, right] = HINT_PADDING.map(px);
    let height = top + px(HINT_LINE) + bottom + px(2.0) + pointer_h;
    let above = caret.top() - callout - height >= px(0.0);
    let fill_color = theme.color(UiRole::SurfaceRaised);
    let border_color = theme.color(UiRole::PopupBorder);
    let text = div()
        .id("editor-read-only-hint")
        .debug_selector(|| "editor-read-only-hint".into())
        .pt(top)
        .pl(left)
        .pb(bottom)
        .pr(right)
        .rounded(px(HINT_RADIUS))
        .border_1()
        .border_color(border_color)
        .bg(fill_color)
        .text_color(theme.color(UiRole::Text))
        .text_size(theme.ui_size())
        .line_height(px(HINT_LINE))
        .shadow(ui::components::popup_shadow())
        .whitespace_nowrap()
        .child(READ_ONLY_HINT);
    // The pointer, over the box's border where it joins the box.
    let pointer = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (x0, x1) = (bounds.left(), bounds.left() + pointer_w);
            let mid = bounds.left() + pointer_w / 2.0;
            let (base, tip) = if above {
                (bounds.top() - px(1.0), bounds.bottom())
            } else {
                (bounds.bottom() + px(1.0), bounds.top())
            };
            let mut fill = PathBuilder::fill();
            fill.move_to(point(x0, base));
            fill.line_to(point(mid, tip));
            fill.line_to(point(x1, base));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, fill_color);
            }
            let mut edge = PathBuilder::stroke(px(1.0));
            edge.move_to(point(x0, base));
            edge.line_to(point(mid, tip));
            edge.line_to(point(x1, base));
            if let Ok(path) = edge.build() {
                window.paint_path(path, border_color);
            }
        },
    )
    .ml(px(POINTER_LEFT))
    .w(pointer_w)
    .h(pointer_h);
    let x = caret.left() - px(POINTER_LEFT) - pointer_w / 2.0;
    let (balloon, at, corner) = if above {
        (
            div().flex().flex_col().child(text).child(pointer),
            point(x, caret.top() - callout),
            gpui::Corner::BottomLeft,
        )
    } else {
        (
            div().flex().flex_col().child(pointer).child(text),
            point(x, caret.bottom() + callout),
            gpui::Corner::TopLeft,
        )
    };
    Some(
        deferred(anchored().position(at).anchor(corner).child(balloon))
            .with_priority(3)
            .into_any_element(),
    )
}
