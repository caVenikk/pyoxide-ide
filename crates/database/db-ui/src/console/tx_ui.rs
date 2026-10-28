//! The pieces of a console's transaction controls that its two toolbars
//! (above the editor, and in the Services tool window) share: the mode
//! button, Commit and Rollback with the number of pending changes, and the
//! unsafe-query banner. The toolbars open the popup of the mode button
//! themselves, anchored under it.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, CursorStyle, Entity, InteractiveElement, IntoElement,
    ParentElement, Pixels, SharedString, StatefulInteractiveElement, Styled, canvas, div, px,
};
use themes::UiRole;
use ui::components::{DISABLED_ALPHA, Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::menu::PopupOptions;
use ui::theme::ThemeExt;

use super::Console;
use super::transactions::{DismissUnsafe, RunUnsafe};
use crate::icons;

/// The least width of the transaction popup: about that of its longest
/// item, so that the footer wraps to two lines as in the reference.
const POPUP_MIN_WIDTH: f32 = 210.0;

/// How the transaction popup opens.
pub(crate) fn popup_options() -> PopupOptions {
    PopupOptions {
        min_width: Some(POPUP_MIN_WIDTH),
        ..PopupOptions::default()
    }
}

/// The mode button: its text, a drop-down arrow, the hover. `bounds`
/// receives where it is drawn (the popup opens under it).
pub(crate) fn mode_button(
    id: &'static str,
    label: impl Into<SharedString>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .relative()
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .h(px(24.0))
        .px(px(6.0))
        .rounded(px(4.0))
        .text_size(px(12.0))
        .whitespace_nowrap()
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| s.bg(hover))
        .child(label.into())
        .child(Icon::new(IconName::CHEVRON_DOWN).size(px(12.0)))
        .tooltip(Tooltip::text("Transaction Mode and Isolation", None))
        .child(
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
}

/// The mode button of the Services tool window's strip: a small square
/// icon button (the reference draws `Tx` with a corner mark, no text).
pub(crate) fn mode_icon_button(
    id: &'static str,
    size: Pixels,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    icon_button(id, icons::TX, size, px(16.0), false, cx)
        .debug_selector(move || id.to_owned())
        .relative()
        .tooltip(Tooltip::text("Transaction Mode and Isolation", None))
        .child(
            canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
}

/// Commit and Rollback, with the pending count after them: shown in manual
/// mode, available while something is pending.
pub(crate) fn commit_buttons(
    prefix: &'static str,
    console: &Entity<Console>,
    button: Pixels,
    icon: Pixels,
    cx: &mut App,
) -> Vec<AnyElement> {
    let c = console.read(cx);
    if !c.is_manual() {
        return Vec::new();
    }
    let pending = c.pending();
    let live = pending > 0 || c.tx.open;
    let commit_id = SharedString::from(format!("{prefix}-commit"));
    let rollback_id = SharedString::from(format!("{prefix}-rollback"));
    let accent = cx.theme().color(UiRole::Accent);
    let commit_tip = match pending {
        0 => "Commit".to_owned(),
        1 => "Commit (1 pending change)".to_owned(),
        n => format!("Commit ({n} pending changes)"),
    };
    let mut out = Vec::new();
    let commit_selector = commit_id.clone();
    let commit = icon_button(commit_id, icons::STATEMENT_OK, button, icon, false, cx)
        .debug_selector(move || commit_selector.to_string())
        .tooltip(Tooltip::text(commit_tip, None));
    out.push(if live {
        let console = console.clone();
        commit
            .on_click(move |_: &ClickEvent, _, cx| console.update(cx, |c, cx| c.commit(cx)))
            .into_any_element()
    } else {
        commit.opacity(DISABLED_ALPHA).into_any_element()
    });
    let rollback_selector = rollback_id.clone();
    let rollback = icon_button(rollback_id, IconName::ROLLBACK, button, icon, false, cx)
        .debug_selector(move || rollback_selector.to_string())
        .tooltip(Tooltip::text("Rollback", None));
    out.push(if live {
        let console = console.clone();
        rollback
            .on_click(move |_: &ClickEvent, _, cx| console.update(cx, |c, cx| c.rollback(cx)))
            .into_any_element()
    } else {
        rollback.opacity(DISABLED_ALPHA).into_any_element()
    });
    if pending > 0 {
        let counter_id = SharedString::from(format!("{prefix}-pending"));
        out.push(
            div()
                .id(counter_id.clone())
                .debug_selector(move || counter_id.to_string())
                .flex_none()
                .px(px(4.0))
                .text_size(px(11.0))
                .text_color(accent)
                .child(pending.to_string())
                .into_any_element(),
        );
    }
    out
}

/// The banner under the toolbar while a statement without `WHERE` waits
/// for the person: the message and its Execute, Execute and Suppress and
/// close.
pub(crate) fn unsafe_banner(message: &str, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let link = theme.color(UiRole::TextLink);
    let link = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .debug_selector(|| id.into())
            .flex_none()
            .whitespace_nowrap()
            .text_color(link)
            .cursor(CursorStyle::PointingHand)
            .child(text)
    };
    div()
        .id("console-unsafe-banner")
        .debug_selector(|| "console-unsafe-banner".into())
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .h(px(36.0))
        .mx(px(4.0))
        .mt(px(4.0))
        .px(px(12.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.color(UiRole::Warning).opacity(0.35))
        .bg(theme.color(UiRole::WarningBg))
        .child(Icon::new(IconName::WARNING).size(px(16.0)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .whitespace_nowrap()
                .child(SharedString::from(message.to_owned())),
        )
        .child(
            link("console-unsafe-execute", "Execute").on_click(|_, window, cx| {
                window.dispatch_action(
                    Box::new(RunUnsafe {
                        suppress: Some(false),
                    }),
                    cx,
                )
            }),
        )
        .child(
            link("console-unsafe-suppress", "Execute and Suppress").on_click(|_, window, cx| {
                window.dispatch_action(
                    Box::new(RunUnsafe {
                        suppress: Some(true),
                    }),
                    cx,
                )
            }),
        )
        .child(
            icon_button(
                "console-unsafe-close",
                IconName::CLOSE_SMALL,
                px(20.0),
                px(14.0),
                false,
                cx,
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(DismissUnsafe), cx)),
        )
        .into_any_element()
}
