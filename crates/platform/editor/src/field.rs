//! One-line text fields of forms (`JBTextField`, `ExtendableTextField`,
//! `TextFieldWithBrowseButton`): an [`Editor`] in one-line mode inside
//! the input frame of `ui::components`, as the form widgets of `ui::form`
//! lay them out.

use gpui::{Action, App, AppContext as _, Div, ElementId, Entity, SharedString, Window, div};
use gpui::{Focusable as _, prelude::*, px};
use ui::components::input_frame;
use ui::form::FIELD_HEIGHT;

use crate::{Editor, EditorConfig};

/// A one-line text field holding `text`: transparent (the frame draws the
/// field), with a placeholder, in the UI font or in the editor font
/// (paths, parameters and the like), without bracket pairing.
pub fn new(
    text: &str,
    placeholder: Option<SharedString>,
    ui_font: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Editor> {
    cx.new(|cx| {
        let mut e = Editor::new(text::Buffer::new(text), window, cx);
        e.set_single_line(true);
        e.set_ui_font(ui_font);
        e.set_transparent(true);
        e.set_placeholder(placeholder);
        e.set_config(EditorConfig {
            brackets: Vec::new(),
            ..EditorConfig::default()
        });
        e
    })
}

/// The field's frame: the input outline (the focus ring while the field
/// has the focus, the danger colour with `error`), the field's height and
/// insets, and the editor one line high. Buttons inside the field
/// ([`ui::form::field_button`]) go after it as children.
pub fn frame(
    editor: &Entity<Editor>,
    ui_font: bool,
    error: bool,
    window: &Window,
    cx: &App,
) -> Div {
    let focused = editor.focus_handle(cx).is_focused(window);
    input_frame(focused, error, cx)
        .h(px(FIELD_HEIGHT))
        .pl(px(8.0))
        .pr(px(7.0))
        .gap(px(9.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .h(crate::field_line_height_for(ui_font, window, cx))
                .overflow_hidden()
                .child(editor.clone()),
        )
}

/// A path field with its Browse button (`TextFieldWithBrowseButton`);
/// without an action the button is drawn disabled.
pub fn browse_frame(
    editor: &Entity<Editor>,
    error: bool,
    browse_id: impl Into<ElementId>,
    browse: Option<Box<dyn Action>>,
    window: &Window,
    cx: &App,
) -> Div {
    frame(editor, false, error, window, cx).child(ui::form::browse_button(browse_id, browse))
}
