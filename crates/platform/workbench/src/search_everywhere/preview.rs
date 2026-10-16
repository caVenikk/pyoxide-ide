//! The preview under the list (the reference's Preview toggle,
//! `PreviewAction`, off by default and remembered,
//! `showPreviewInSearchEverywhere`): the selected row's file in the shared
//! read-only preview ([`crate::file_preview`]), a file row at its start (at
//! the line typed after its name), a text row at its line with the matches
//! marked, shown [`PREVIEW_DELAY`] after the selection settles. A row that
//! is not a place in a file (a folder, an action) shows "There is no
//! preview for the selected item"; without a selected row (or on "… more")
//! there is no preview and the list takes the room.
//!
//! The list and the preview share the popup's height with a splitter
//! between them, the list a third by default (the reference's
//! `OnePixelSplitter(true, .33f)`, its proportion remembered). The Actions
//! tab has no preview (the reference's tab turns it off).

use file_index::parse_position;
use gpui::{
    AppContext as _, Bounds, Context, CursorStyle, Entity, HitboxBehavior, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels, Styled as _,
    Subscription, Task, Window, canvas, div, px,
};
use text::Point;
use themes::UiRole;
use ui::theme::ThemeExt as _;

use super::{Row, SearchEverywhere, Tab, settings, update_settings};
use crate::file_preview::{FilePreview, FilePreviewEvent, NO_PREVIEW, PreviewOptions};
use crate::find_in_files::PREVIEW_DELAY;
use crate::popup::{HeldDrag, track_held_drag};

/// Share of the list in the split with the preview (the reference's).
pub const SPLIT: f32 = 0.33;
/// Smallest height of the list and of the preview.
const MIN_PART: f32 = 48.0;

pub(super) struct Preview {
    pub(super) view: Entity<FilePreview>,
    /// The wait for the selection to settle.
    task: Option<Task<()>>,
    /// Share of the list in the split, the split's drag in progress
    /// (start y, share) and its bounds.
    split: f32,
    drag: HeldDrag<(Pixels, f32)>,
    bounds: Option<Bounds<Pixels>>,
    _subscription: Subscription,
}

impl Preview {
    pub fn new(
        file_types: std::rc::Rc<crate::file_types::FileTypes>,
        window: &mut Window,
        cx: &mut Context<SearchEverywhere>,
    ) -> Self {
        let options = PreviewOptions {
            editable: false,
            soft_wrap_button: true,
            empty_text: NO_PREVIEW.into(),
            name: "search-everywhere-preview",
        };
        let view = cx.new(|cx| FilePreview::new(options, file_types, window, cx));
        let subscription = cx.subscribe(&view, |this, _, event: &FilePreviewEvent, cx| {
            if let FilePreviewEvent::Loaded(path) = event {
                this.preview_loaded(path, cx);
            }
        });
        Self {
            view,
            task: None,
            split: settings(cx).preview_split.unwrap_or(SPLIT).clamp(0.1, 0.9),
            drag: HeldDrag::default(),
            bounds: None,
            _subscription: subscription,
        }
    }

    /// Share of the list in the split.
    pub(super) fn split(&self) -> f32 {
        self.split
    }

    /// The splitter is dragged.
    pub(super) fn is_dragging(&self) -> bool {
        self.drag.is_dragging()
    }
}

impl SearchEverywhere {
    /// The main window (its tabs' texts are shown for open files).
    pub fn set_host(
        &mut self,
        host: gpui::WeakEntity<crate::workbench::Workbench>,
        cx: &mut Context<Self>,
    ) {
        self.preview.view.update(cx, |p, _| p.set_host(host));
    }

    /// The preview's editor.
    pub fn preview_editor(&self, cx: &gpui::App) -> Entity<editor::Editor> {
        self.preview.view.read(cx).editor().clone()
    }

    /// The Preview toggle is on and the tab has a preview.
    pub(super) fn preview_on(&self, cx: &gpui::App) -> bool {
        settings(cx).preview && self.tab != Tab::Actions
    }

    /// The preview is shown under the list: the toggle is on and an item
    /// is selected.
    pub(super) fn preview_shown(&self, cx: &gpui::App) -> bool {
        self.preview_on(cx)
            && self
                .selected
                .and_then(|i| self.rows.get(i))
                .is_some_and(|r| *r != Row::More)
    }

    /// Preview on or off (remembered).
    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        update_settings(cx, |s| s.preview = !s.preview);
        self.preview_selected(cx);
        cx.notify();
    }

    /// Shows the selected row in the preview once the selection settles.
    pub(super) fn preview_selected(&mut self, cx: &mut Context<Self>) {
        if !self.open || !self.preview_on(cx) {
            self.preview.task = None;
            return;
        }
        self.preview.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PREVIEW_DELAY).await;
            let _ = this.update(cx, |this, cx| this.show_preview(cx));
        }));
    }

    /// The popup closed: the preview lets go of its file.
    pub(super) fn clear_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.task = None;
        self.preview.drag.end();
        self.preview.view.update(cx, |p, cx| p.clear(cx));
    }

    fn show_preview(&mut self, cx: &mut Context<Self>) {
        self.preview.task = None;
        let row = self.selected.and_then(|i| self.rows.get(i)).cloned();
        let path = match &row {
            Some(Row::File(f)) if !f.is_dir => f.path.clone(),
            Some(Row::Text(t)) => t.path.clone(),
            Some(Row::File(_)) | Some(Row::Action(_)) => {
                self.preview.view.update(cx, |p, cx| p.show_empty(cx));
                return;
            }
            Some(Row::More) | None => {
                self.preview.view.update(cx, |p, cx| p.clear(cx));
                return;
            }
        };
        let shown = self.preview.view.update(cx, |p, cx| p.show_file(path, cx));
        if shown {
            self.mark_preview(cx);
        }
    }

    fn preview_loaded(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let selected = match self.selected.and_then(|i| self.rows.get(i)) {
            Some(Row::File(f)) => f.path == path,
            Some(Row::Text(t)) => t.path == path,
            _ => false,
        };
        if selected {
            self.mark_preview(cx);
        }
    }

    /// The selected row's place in the preview: a file at its start (or
    /// the line and column typed after its name), a text row at its
    /// matches, its line in the middle.
    fn mark_preview(&mut self, cx: &mut Context<Self>) {
        let row = self.selected.and_then(|i| self.rows.get(i)).cloned();
        let text = self.field.read(cx).text();
        let view = self.preview.view.clone();
        let buffer = view.read(cx).editor().read(cx).buffer().clone();
        let lines = buffer.len_lines();
        let (ranges, line, at) = match row {
            Some(Row::File(_)) => {
                let position = parse_position(&text);
                let at = Point::new(
                    position.line.unwrap_or(0).min(lines.saturating_sub(1)),
                    position.column.unwrap_or(0),
                );
                (Vec::new(), None, at)
            }
            Some(Row::Text(t)) => {
                let line = t.line.min(lines.saturating_sub(1));
                let start = buffer.line_start(line);
                let len = buffer.line_len(line);
                // The row's text is the line without its indentation.
                let indent = t
                    .ranges
                    .first()
                    .map_or(0, |r| t.column.saturating_sub(r.start));
                let ranges: Vec<_> = t
                    .ranges
                    .iter()
                    .map(|r| start + (indent + r.start).min(len)..start + (indent + r.end).min(len))
                    .collect();
                (ranges, Some(line), Point::new(line, t.column.min(len)))
            }
            _ => return,
        };
        view.update(cx, |p, cx| p.mark(&ranges, None, line, Some(at), cx));
    }

    // ---- drawing ------------------------------------------------------------

    /// The list over the preview with the splitter between them.
    pub(super) fn render_split(
        &self,
        list: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let share = self.preview.split;
        let this = cx.entity();
        let measure = canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| this.preview.bounds = Some(bounds));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let splitter = div()
            .id("search-everywhere-splitter")
            .debug_selector(|| "search-everywhere-splitter".into())
            .relative()
            .flex_none()
            .h(px(1.0))
            .bg(theme.color(UiRole::Divider))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(-3.0))
                    .h(px(7.0))
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            |_, hitbox, window, _| {
                                ui::pointer::request(CursorStyle::ResizeUpDown, &hitbox, window)
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.preview.drag.start((e.position.y, share));
                            cx.notify();
                        }),
                    ),
            );
        // The split is kept once the drag ends.
        let tracking = track_held_drag(
            cx.entity(),
            |this: &mut Self| &mut this.preview.drag,
            |this, mouse, cx| this.drag_split(mouse.y, cx),
            |this, cx| {
                let split = this.preview.split;
                update_settings(cx, |s| s.preview_split = Some(split));
            },
            self.preview
                .drag
                .is_dragging()
                .then_some(CursorStyle::ResizeUpDown),
        );
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .child(measure)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_grow()
                    .flex_basis(gpui::relative(share))
                    .min_h(px(MIN_PART))
                    .child(list),
            )
            .child(splitter)
            .child(
                div()
                    .id("search-everywhere-preview-pane")
                    .debug_selector(|| "search-everywhere-preview-pane".into())
                    .flex()
                    .flex_col()
                    .flex_grow()
                    .flex_basis(gpui::relative(1.0 - share))
                    .min_h(px(MIN_PART))
                    .overflow_hidden()
                    .child(self.preview.view.clone()),
            )
            .child(tracking)
            .into_any_element()
    }

    fn drag_split(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let (Some((start, share)), Some(bounds)) = (self.preview.drag.get(), self.preview.bounds)
        else {
            return;
        };
        let height = f32::from(bounds.size.height).max(1.0);
        let moved = f32::from(y - start) / height;
        self.preview.split = (share + moved).clamp(0.1, 0.9);
        cx.notify();
    }
}
