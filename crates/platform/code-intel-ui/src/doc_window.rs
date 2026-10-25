//! The Documentation tool window (`DocumentationToolWindowManager`: the
//! right area, its stripe button added by its first use): Open in
//! Documentation Tool Window moves the popup's documentation into it, and
//! Quick Documentation shows there at once while Show Documentation Popup
//! First is off. It shows the last documentation, drawn as the popup
//! draws it without its frame; the workbench registers and shows it on
//! [`crate::CodeIntelEvent::ShowDocumentation`].
//!
//! Not yet: tabs (Keep This Documentation), Auto-Update from Source, the
//! title's Back, Forward and Jump to Source, and the gear's options.

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, ParentElement, Render, Styled, WeakEntity, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};
use ui::{IconName, ThemeExt};

use crate::doc::{DocContent, DocKind, DocPopup, Documentation};
use crate::view::CodeIntelView;

/// The tool window's id (`documentation.v2` in the reference).
pub const ID: &str = "documentation";
/// Its stripe title (`toolwindow.stripe.documentation.v2`).
pub const TITLE: &str = "Documentation";

const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;

/// The view of the Documentation tool window.
pub struct DocToolWindow {
    focus: FocusHandle,
    content: Entity<DocPopup>,
}

impl Focusable for DocToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DocToolWindow {
    pub fn new(owner: WeakEntity<CodeIntelView>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: cx.new(|_| DocPopup::for_tool_window(owner)),
        }
    }

    /// Shows `documentation` in place of what it showed.
    pub fn set(&mut self, documentation: Documentation, cx: &mut Context<Self>) {
        let content = DocContent {
            kind: DocKind::QuickDoc,
            problems: Vec::new(),
            fix: None,
            more_actions: false,
            documentation: Some(documentation),
        };
        self.content.update(cx, |p, cx| p.set(content, cx));
        cx.notify();
    }

    /// The documentation shown, for `code.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        let content = self.content.read(cx).content();
        json!({
            "title": TITLE,
            "documentation": content.map(DocContent::to_json),
        })
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focus = self.focus.clone();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(theme.metric(Metric::ToolWindowHeaderHeight))
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child(TITLE),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "documentation-hide",
                    IconName::HIDE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .debug_selector(|| "documentation-hide".into())
                .tooltip(Tooltip::text("Hide", None))
                // Hide Active Tool Window hides the one with the focus.
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    if let Ok(hide) = cx.build_action("workbench.hide-tool-window", None) {
                        window.dispatch_action(hide, cx);
                    }
                }),
            )
    }
}

impl Render for DocToolWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("documentation-tool-window")
            .key_context("DocumentationToolWindow")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.color(UiRole::Text))
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .child(self.content.clone()),
            )
    }
}
