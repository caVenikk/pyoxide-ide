//! The Export Data dialog (shots DB76 to DB79): the source table, the data
//! extractor, the options that apply to the tabular extractors, the output
//! file and a preview of the first rows. The preview is made by the export
//! job itself (to memory, ten rows), so it shows exactly what the file will
//! hold. "Export to File" and "Copy to Clipboard" close the dialog and run
//! the job in the background ([`super::run`]).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use db_session::{
    ExportEvent, ExportOptions, ExportOutcome, ExportRequest, ExportSink, ExportSource,
    ExportTarget, Format, JobId,
};
use editor::Editor;
use futures::StreamExt as _;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle, Focusable,
    IntoElement, PathPromptOptions, Pixels, Render, SharedString, Subscription, Task, Window,
    canvas, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use text::{Selection, SelectionSet};
use themes::UiRole;
use ui::form::{self, BarButton, BottomBar, ButtonRole, MenuPopup};
use ui::menu::MenuItem;
use ui::theme::ThemeExt;

use super::run::{self, Goal};
use super::{
    ExportBrowse, ExportCancel, ExportFormatMenu, ExportSetFormat, ExportSetNullText,
    ExportSetOption, ExportSetPath, ExportToClipboard, ExportToFile, TransferTarget,
};

const TITLE: &str = "Export Data";
const WIDTH: f32 = 830.0;
const HEIGHT: f32 = 556.0;
const COLUMN: f32 = 400.0;
const PREVIEW_ROWS: u64 = 10;
const KEY_CONTEXT: &str = "DatabaseExportDialog";

/// The dialog's view. It holds the table, the choices and the preview.
pub(crate) struct ExportDialog {
    target: TransferTarget,
    /// The main window the export's balloon goes to.
    parent: AnyWindowHandle,
    format: Format,
    transpose: bool,
    column_header: bool,
    row_header: bool,
    path: Entity<Editor>,
    /// How NULL is written (tabular extractors).
    null_text: Entity<Editor>,
    /// The path the dialog wrote itself; the field differing from it was
    /// edited by the user, and then the extension is left alone.
    auto_path: String,
    preview: Entity<Editor>,
    preview_job: Option<JobId>,
    preview_error: Option<String>,
    sink: ExportSink,
    list: Option<MenuPopup>,
    combo: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    _events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ExportDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_path(table: &str, format: Format) -> String {
    home()
        .join(format!("{table}.{}", format.extension()))
        .display()
        .to_string()
}

/// Opens the dialog over the main window `window`.
pub(crate) fn open(target: TransferTarget, window: &mut Window, cx: &mut App) {
    let options = ui::dialog_window::DialogOptions {
        title: TITLE.into(),
        size: gpui::size(
            px(WIDTH),
            px(HEIGHT + ui::dialog_window::title_height(true)),
        ),
        activate: !workbench::env::background(),
        themed_title: true,
        ..ui::dialog_window::DialogOptions::default()
    };
    let parent = window.window_handle();
    let mut built = None;
    ui::dialog_window::open(window, options, cx, |window, cx| {
        let dialog = cx.new(|cx| ExportDialog::new(target, parent, window, cx));
        built = Some(dialog.clone());
        dialog
    });
    if let Some(dialog) = built {
        super::set_open_export(Some(dialog.downgrade()), cx);
    }
}

impl ExportDialog {
    fn new(
        target: TransferTarget,
        parent: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let format = Format::Csv;
        let auto_path = default_path(&target.target.table.name, format);
        let path = editor::field::new(&auto_path, None, false, window, cx);
        let preview = cx.new(|cx| {
            let mut e = Editor::new(text::Buffer::new(""), window, cx);
            e.set_viewer(true);
            e
        });
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<ExportEvent>();
        let sink: ExportSink = Arc::new(move |e| {
            let _ = tx.unbounded_send(e);
        });
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = rx.next().await {
                if this.update(cx, |d, cx| d.on_event(event, cx)).is_err() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        let null_text = editor::field::new("", Some("empty".into()), false, window, cx);
        let subscriptions = vec![
            cx.observe(&path, |_, _, cx| cx.notify()),
            cx.subscribe(&null_text, |this, _, e: &editor::EditorEvent, cx| {
                if matches!(e, editor::EditorEvent::Edited(_)) {
                    this.refresh_preview(cx);
                }
            }),
        ];
        let mut dialog = Self {
            target,
            parent,
            format,
            transpose: false,
            column_header: false,
            row_header: false,
            path,
            null_text,
            auto_path,
            preview,
            preview_job: None,
            preview_error: None,
            sink,
            list: None,
            combo: Rc::default(),
            focus,
            _events: events,
            _subscriptions: subscriptions,
        };
        window.focus(&dialog.focus);
        dialog.refresh_preview(cx);
        dialog
    }

    // ---- choices -----------------------------------------------------------------

    fn options(&self, cx: &App) -> ExportOptions {
        let mut options = ExportOptions::new(self.format, self.target.dialect());
        options.column_header = self.column_header;
        options.row_header = self.row_header;
        options.transpose = self.transpose;
        options.null_text = self.null_text.read(cx).text();
        options.key_columns = self.target.key_columns();
        options
    }

    fn request(&self, target: ExportTarget, limit: Option<u64>, cx: &App) -> ExportRequest {
        ExportRequest {
            source: ExportSource::Table(self.target.query.clone()),
            options: self.options(cx),
            target,
            limit,
        }
    }

    fn path_text(&self, cx: &App) -> String {
        self.path.read(cx).text()
    }

    fn set_format(&mut self, format: Format, cx: &mut Context<Self>) {
        self.close_list(cx);
        if self.format == format {
            return;
        }
        let current = self.path_text(cx);
        self.format = format;
        // A path the dialog suggested follows the extractor's extension.
        if current == self.auto_path {
            let next = default_path(&self.target.target.table.name, format);
            self.auto_path = next.clone();
            self.path.update(cx, |e, cx| e.set_text_selected(&next, cx));
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    fn set_option(&mut self, option: &str, value: bool, cx: &mut Context<Self>) {
        match option {
            "transpose" => self.transpose = value,
            "column-header" => self.column_header = value,
            "row-header" => self.row_header = value,
            _ => return,
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    fn set_null_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.null_text
            .update(cx, |e, cx| e.set_text_selected(text, cx));
        self.refresh_preview(cx);
        cx.notify();
    }

    fn set_path(&mut self, path: &str, cx: &mut Context<Self>) {
        self.path.update(cx, |e, cx| e.set_text_selected(path, cx));
        cx.notify();
    }

    // ---- preview -----------------------------------------------------------------

    /// Reads the first rows again with the choices made.
    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.preview_job.take() {
            self.target.target.handle.cancel_job(job);
        }
        let request = self.request(ExportTarget::Memory, Some(PREVIEW_ROWS), cx);
        self.preview_job = Some(self.target.target.handle.export(request, self.sink.clone()));
    }

    fn on_event(&mut self, event: ExportEvent, cx: &mut Context<Self>) {
        let ExportEvent::Finished { job, outcome } = event else {
            return;
        };
        if self.preview_job != Some(job) {
            return;
        }
        self.preview_job = None;
        match outcome {
            ExportOutcome::Done(done) => {
                self.preview_error = None;
                self.show(&done.text.unwrap_or_default(), cx);
            }
            ExportOutcome::Failed(e) => {
                self.preview_error = Some(e.message);
                self.show("", cx);
            }
            ExportOutcome::Cancelled => {}
        }
        cx.notify();
    }

    /// Puts `text` into the preview with the caret at its start.
    fn show(&mut self, text: &str, cx: &mut Context<Self>) {
        self.preview.update(cx, |e, cx| {
            e.set_text_selected(text, cx);
            e.set_selections(SelectionSet::single(Selection::new(0, 0)), cx);
        });
    }

    // ---- running -----------------------------------------------------------------

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(job) = self.preview_job.take() {
            self.target.target.handle.cancel_job(job);
        }
        self.close_list(cx);
        super::set_open_export(None, cx);
        ui::dialog_window::close(window.window_handle(), cx);
        window.remove_window();
    }

    fn run(&mut self, goal: Goal, window: &mut Window, cx: &mut Context<Self>) {
        let target = match goal {
            Goal::File => {
                let text = self.path_text(cx);
                let path = PathBuf::from(text.trim());
                if text.trim().is_empty() || path.is_dir() {
                    return;
                }
                ExportTarget::File(path)
            }
            Goal::Clipboard => ExportTarget::Memory,
        };
        let request = self.request(target, None, cx);
        let transfer = self.target.clone();
        let parent = self.parent;
        self.close(window, cx);
        cx.defer(move |cx| run::export(&transfer, request, goal, parent, cx));
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = Path::new(self.path_text(cx).trim())
            .file_name()
            .map(|n| n.to_owned())
            .unwrap_or_else(|| default_path(&self.target.target.table.name, self.format).into());
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |d, cx| {
                d.set_path(&dir.join(name).display().to_string(), cx)
            });
        })
        .detach();
    }

    // ---- the extractor list ------------------------------------------------------

    fn open_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(cx);
        let items: Vec<MenuItem> = Format::ALL
            .into_iter()
            .map(|f| {
                MenuItem::action(
                    f.title(),
                    ExportSetFormat {
                        format: f.id().to_owned(),
                    },
                )
                .bold(f == self.format)
            })
            .collect();
        let bounds = self.combo.get();
        let position = point(bounds.left(), bounds.bottom() + px(2.0));
        let activate = !workbench::env::background();
        self.list = MenuPopup::open(
            items,
            None,
            position,
            &self.focus,
            activate,
            window,
            cx,
            |this, cx| this.close_list(cx),
        );
        cx.notify();
    }

    fn close_list(&mut self, cx: &mut Context<Self>) {
        if let Some(list) = self.list.take() {
            list.close(cx);
        }
    }

    pub(crate) fn state_json(&self, cx: &App) -> Json {
        json!({
            "source": self.target.title(),
            "extractor": self.format.title(),
            "format": self.format.id(),
            "transpose": self.transpose,
            "column_header": self.column_header,
            "row_header": self.row_header,
            "path": self.path_text(cx),
            "null_text": self.null_text.read(cx).text(),
            "preview": self.preview.read(cx).text(),
            "preview_loading": self.preview_job.is_some(),
            "preview_error": self.preview_error,
            "list_open": self.list.is_some(),
        })
    }

    // ---- drawing -----------------------------------------------------------------

    fn label(text: &'static str, cx: &App) -> gpui::Div {
        div().text_color(cx.theme().color(UiRole::Text)).child(text)
    }

    /// The text NULL is written as (not in the reference's dialog: the
    /// reference keeps it in the extractor's settings).
    fn null_row(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let enabled = self.format.is_tabular();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .child(div().flex_none().child("NULL text:"))
            .child(div().flex_1().child(editor::field::frame(
                &self.null_text,
                false,
                false,
                window,
                cx,
            )))
            .into_any_element()
    }

    fn option(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let enabled = self.format.is_tabular();
        form::check_row(
            SharedString::from(format!("export-{id}")),
            SharedString::from(format!("export-{id}-box")),
            on && enabled,
            div().child(label),
            cx,
        )
        .debug_selector(move || format!("export-{id}"))
        .h(px(form::FIELD_HEIGHT))
        .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
        .when(enabled, |d| {
            d.on_click(cx.listener(move |this, _, _, cx| this.set_option(id, !on, cx)))
        })
        .into_any_element()
    }
}

impl Render for ExportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (surface, text_color, danger) = (
            theme.color(UiRole::Surface),
            theme.color(UiRole::Text),
            theme.color(UiRole::Danger),
        );
        let (editor_bg, ui_size, ui_family) = (
            theme.color(UiRole::EditorBackground),
            theme.ui_size(),
            theme.ui_family(),
        );
        let border = theme.color(UiRole::BorderInput);
        let combo_bounds = self.combo.clone();
        let combo = form::combo(
            "export-extractor",
            vec![div().child(self.format.title()).into_any_element()],
            cx,
        )
        .debug_selector(|| "export-extractor".to_owned())
        .w(px(300.0))
        .child(
            canvas(move |b, _, _| combo_bounds.set(b), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .on_click(cx.listener(|this, _, window, cx| this.open_list(window, cx)));
        let source = ui::components::input_frame(false, false, cx)
            .h(px(form::FIELD_HEIGHT))
            .px(px(8.0))
            .child(self.target.title());
        let path = editor::field::browse_frame(
            &self.path,
            false,
            "export-browse",
            Some(Box::new(ExportBrowse)),
            window,
            cx,
        );
        let preview_text: Option<String> = self.preview_error.clone();
        let preview = div()
            .flex_1()
            .min_h(px(0.0))
            .border_1()
            .border_color(border)
            .rounded(form::field_radius(cx))
            .overflow_hidden()
            .bg(editor_bg)
            .child(self.preview.clone())
            .when_some(preview_text, |d, error| {
                d.child(
                    div()
                        .absolute()
                        .top(px(8.0))
                        .left(px(8.0))
                        .text_color(danger)
                        .child(error),
                )
            });
        let left = div()
            .w(px(COLUMN))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Self::label("Source:", cx))
            .child(source)
            .child(div().h(px(6.0)))
            .child(Self::label("Extractor:", cx))
            .child(combo)
            .child(self.option("transpose", "Transpose", self.transpose, cx))
            .child(self.option("column-header", "Add column header", self.column_header, cx))
            .child(self.option("row-header", "Add row header", self.row_header, cx))
            .child(self.null_row(window, cx))
            .child(Self::label("Output file:", cx))
            .child(path);
        let right = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Self::label("Export preview (first 10 rows are shown):", cx))
            .child(preview);
        let bar = BottomBar::new()
            .button(BarButton::new(
                "export-cancel",
                "Cancel",
                ButtonRole::Cancel,
                Box::new(ExportCancel),
            ))
            .button(BarButton::new(
                "export-clipboard",
                "Copy to Clipboard",
                ButtonRole::Other,
                Box::new(ExportToClipboard),
            ))
            .button(BarButton::new(
                "export-file",
                "Export to File",
                ButtonRole::Ok,
                Box::new(ExportToFile),
            ))
            .render(cx);
        div()
            .id("export-dialog")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, a: &ExportSetFormat, _, cx| {
                if let Some(format) = Format::from_id(&a.format) {
                    this.set_format(format, cx);
                }
            }))
            .on_action(cx.listener(|this, a: &ExportSetOption, _, cx| {
                this.set_option(&a.option, a.value, cx)
            }))
            .on_action(
                cx.listener(|this, a: &ExportSetNullText, _, cx| this.set_null_text(&a.text, cx)),
            )
            .on_action(cx.listener(|this, a: &ExportSetPath, _, cx| this.set_path(&a.path, cx)))
            .on_action(cx.listener(|this, _: &ExportBrowse, window, cx| this.browse(window, cx)))
            .on_action(
                cx.listener(|this, _: &ExportFormatMenu, window, cx| this.open_list(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ExportToFile, window, cx| this.run(Goal::File, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExportToClipboard, window, cx| {
                this.run(Goal::Clipboard, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportCancel, window, cx| this.close(window, cx)))
            .on_key_down(cx.listener(|this, e: &gpui::KeyDownEvent, window, cx| {
                match e.keystroke.key.as_str() {
                    "enter" => this.run(Goal::File, window, cx),
                    "escape" => this.close(window, cx),
                    _ => {}
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(surface)
            .text_color(text_color)
            .text_size(ui_size)
            .font_family(ui_family)
            .children(ui::dialog_window::title_bar(TITLE, true, cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_row()
                    .gap(px(14.0))
                    .px(px(16.0))
                    .pt(px(12.0))
                    .pb(px(10.0))
                    .child(left)
                    .child(right),
            )
            .child(bar)
            .child(ui::pointer::frame())
            .children(ui::dialog_window::blocker(window, cx))
    }
}
