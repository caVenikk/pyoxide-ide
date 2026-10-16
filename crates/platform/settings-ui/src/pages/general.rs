//! Editor > General (`EditorOptionsPanel`), the Soft Wraps group only
//! (`[editor.soft-wraps]`): which files open with soft wraps, the indent of
//! wrapped fragments and where the soft wrap signs show. The open editors
//! follow the store when the page is applied.
//!
//! Not drawn yet: the reference's other groups (Mouse Control, Virtual
//! Space, Scroll Offset, Limits, Rich-Text Copy, Highlight on Caret
//! Movement, Formatting, ...).

use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FocusHandle, IntoElement, Render,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings::{DEFAULT_SOFT_WRAP_FILES, SoftWrapSettings};
use themes::UiRole;
use ui::components::DISABLED_ALPHA;
use ui::theme::ThemeExt;

use crate::page::{PageContext, SettingsPage};
use crate::store;

pub const ID: &str = "editor.general";
pub const TITLE: &str = "General";

/// The reference's labels (ApplicationBundle).
const SOFT_WRAPS: &str = "Soft Wraps";
const SOFT_WRAP_FILES: &str = "Soft-wrap these files:";
const NO_FILES: &str = "No files";
const MASKS_HINT: &str = "Use * and ? as wildcards and ; to separate patterns";
const ORIGINAL_INDENT: &str = "Use the original line's indent for wrapped fragments";
const ADDITIONAL_INDENT: &str = "Add additional indent:";
const SYMBOLS: &str = "symbols";
const CURRENT_LINE_ONLY: &str = "Only show soft-wrap indicators for the current line";
pub const SEARCH_TERMS: &[&str] = &[
    SOFT_WRAPS,
    SOFT_WRAP_FILES,
    MASKS_HINT,
    ORIGINAL_INDENT,
    ADDITIONAL_INDENT,
    SYMBOLS,
    CURRENT_LINE_ONLY,
];

/// Kotlin UI DSL spacing, as on System Settings: the indent of a group's
/// rows and of an indented row, the gap between rows, rows 22 high, the
/// small gap (`RightGap.SMALL`), the masks field of `COLUMNS_LARGE` and the
/// indent field of two columns, fields 24 high, the comment 4 under its
/// field.
const GROUP_INDENT: f32 = 20.0;
const ROW_GAP: f32 = 6.0;
const ROW_HEIGHT: f32 = 22.0;
const SMALL_GAP: f32 = 6.0;
const MASKS_WIDTH: f32 = 280.0;
const INDENT_WIDTH: f32 = 40.0;
const FIELD_HEIGHT: f32 = 24.0;
const COMMENT_TOP: f32 = 4.0;

/// The keys of `settings.edit`.
mod field {
    pub const SOFT_WRAPS: &str = "soft-wraps";
    pub const FILES: &str = "files";
    pub const ORIGINAL_INDENT: &str = "original-indent";
    pub const INDENT: &str = "indent";
    pub const CURRENT_LINE_ONLY: &str = "current-line-only";
}

/// The page's draft: the check boxes and the texts of the fields.
pub struct GeneralPage {
    enabled: bool,
    use_original_indent: bool,
    show_all: bool,
    files: Entity<Editor>,
    indent: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl GeneralPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stored: SoftWrapSettings = store::get(cx);
        let files = editor::field::new(stored.masks(), Some(NO_FILES.into()), true, window, cx);
        let indent = editor::field::new(&stored.indent.to_string(), None, true, window, cx);
        let edited = |_: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
            if matches!(e, EditorEvent::Edited(_)) {
                cx.notify();
            }
        };
        let subscriptions = vec![cx.subscribe(&files, edited), cx.subscribe(&indent, edited)];
        Self {
            enabled: stored.enabled,
            use_original_indent: stored.use_original_indent,
            show_all: stored.show_all,
            files,
            indent,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn text(field: &Entity<Editor>, cx: &App) -> String {
        field.read(cx).text().trim().to_owned()
    }

    /// The draft as settings; an indent that is no number in range is an
    /// error.
    fn draft(&self, cx: &App) -> Result<SoftWrapSettings, String> {
        let range = SoftWrapSettings::INDENTS;
        let indent: usize = Self::text(&self.indent, cx)
            .parse()
            .ok()
            .filter(|n| range.contains(n))
            .ok_or_else(|| {
                format!(
                    "Please enter a number from {} to {}",
                    range.start(),
                    range.end()
                )
            })?;
        // Masks equal to the ones the settings show without masks of their
        // own are not written.
        let text = Self::text(&self.files, cx);
        let implied = SoftWrapSettings {
            enabled: self.enabled,
            files: None,
            ..SoftWrapSettings::default()
        };
        let files = (text != implied.masks()).then_some(text);
        Ok(SoftWrapSettings {
            enabled: self.enabled,
            files,
            use_original_indent: self.use_original_indent,
            indent,
            show_all: self.show_all,
        })
    }

    fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            field::SOFT_WRAPS => self.enabled = !self.enabled,
            field::ORIGINAL_INDENT => self.use_original_indent = !self.use_original_indent,
            field::CURRENT_LINE_ONLY => self.show_all = !self.show_all,
            _ => return,
        }
        cx.notify();
    }

    /// A check box with its label, lit up while the window's search finds
    /// the label.
    fn check(
        &self,
        name: &'static str,
        checked: bool,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = ui::form::check_row(
            ElementId::Name(format!("general-row:{name}").into()),
            ElementId::Name(format!("general-check:{name}").into()),
            checked,
            div().whitespace_nowrap().child(label),
            cx,
        )
        .debug_selector(move || format!("general:{name}"))
        .min_h(px(ROW_HEIGHT))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(name, cx)));
        crate::search::spotlight(label, row, cx)
    }

    fn field(
        &self,
        field: &Entity<Editor>,
        width: f32,
        enabled: bool,
        error: bool,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        div()
            .relative()
            .flex_none()
            .w(px(width))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .child(editor::field::frame(field, true, error, window, cx).h(px(FIELD_HEIGHT)))
            // A disabled field takes no clicks (`enabledIf`).
            .when(!enabled, |d| {
                d.child(
                    div()
                        .id("general-field-disabled")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude(),
                )
            })
            .into_any_element()
    }
}

impl SettingsPage for GeneralPage {
    fn is_modified(&self, cx: &App) -> bool {
        match self.draft(cx) {
            Ok(draft) => draft != store::get::<SoftWrapSettings>(cx),
            Err(_) => true,
        }
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let draft = self.draft(cx)?;
        store::set(cx, &draft);
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let stored: SoftWrapSettings = store::get(cx);
        self.enabled = stored.enabled;
        self.use_original_indent = stored.use_original_indent;
        self.show_all = stored.show_all;
        let masks = stored.masks().to_owned();
        let indent = stored.indent.to_string();
        self.files
            .update(cx, |e, cx| e.set_text_selected(&masks, cx));
        self.indent
            .update(cx, |e, cx| e.set_text_selected(&indent, cx));
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        json!({
            "soft-wraps": self.enabled,
            "files": Self::text(&self.files, cx),
            "files-enabled": self.enabled,
            "original-indent": self.use_original_indent,
            "indent": Self::text(&self.indent, cx),
            "indent-enabled": self.use_original_indent,
            "current-line-only": !self.show_all,
            "default-files": DEFAULT_SOFT_WRAP_FILES,
        })
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let flag = || {
            value
                .as_bool()
                .ok_or_else(|| format!("`{name}` takes true or false"))
        };
        let text = || match value {
            Json::String(s) => Ok(s.clone()),
            Json::Number(n) => Ok(n.to_string()),
            _ => Err(format!("`{name}` takes a string")),
        };
        match name {
            field::SOFT_WRAPS => self.enabled = flag()?,
            field::ORIGINAL_INDENT => self.use_original_indent = flag()?,
            field::CURRENT_LINE_ONLY => self.show_all = !flag()?,
            field::FILES => {
                let t = text()?;
                self.files.update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            field::INDENT => {
                let t = text()?;
                self.indent.update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            _ => return Err(format!("no field `{name}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for GeneralPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let small = theme.ui_size() - px(1.0);
        let indent_error = self.draft(cx).is_err();
        let enabled = self.use_original_indent;
        let dim = |label: AnyElement| {
            div()
                .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
                .child(label)
        };
        let files_row = div()
            .flex()
            .flex_row()
            .items_start()
            .child(
                div()
                    .h(px(FIELD_HEIGHT))
                    .flex()
                    .items_center()
                    .child(self.check(field::SOFT_WRAPS, self.enabled, SOFT_WRAP_FILES, cx)),
            )
            .child(div().w(px(SMALL_GAP)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(self.field(&self.files, MASKS_WIDTH, self.enabled, false, window, cx))
                    .child(
                        div()
                            .pt(px(COMMENT_TOP))
                            .text_size(small)
                            .text_color(muted)
                            .child(crate::search::label(MASKS_HINT, cx)),
                    ),
            );
        let indent_row = div()
            .pl(px(GROUP_INDENT))
            .flex()
            .flex_row()
            .items_center()
            .min_h(px(ROW_HEIGHT))
            .child(dim(crate::search::label(ADDITIONAL_INDENT, cx)))
            .child(div().w(px(SMALL_GAP)))
            .child(self.field(
                &self.indent,
                INDENT_WIDTH,
                self.use_original_indent,
                indent_error,
                window,
                cx,
            ))
            .child(div().w(px(SMALL_GAP)))
            .child(dim(crate::search::label(SYMBOLS, cx)));
        let rows: Vec<AnyElement> = vec![
            files_row.into_any_element(),
            self.check(
                field::ORIGINAL_INDENT,
                self.use_original_indent,
                ORIGINAL_INDENT,
                cx,
            ),
            indent_row.into_any_element(),
            self.check(
                field::CURRENT_LINE_ONLY,
                !self.show_all,
                CURRENT_LINE_ONLY,
                cx,
            ),
        ];
        div()
            .id("general-page")
            .debug_selector(|| "general-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(ui::form::group_header(SOFT_WRAPS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .children(rows),
            )
    }
}
