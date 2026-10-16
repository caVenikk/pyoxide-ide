//! The preview of a file in a popup (the reference's `UsagePreviewPanel`):
//! one component for Find in Files and Search Everywhere, set up with
//! [`PreviewOptions`] when it is created.
//!
//! It shows a file in an editor with its syntax colours; its owner marks
//! ranges and puts the caret on a line in the middle of the view
//! ([`FilePreview::mark`]). When there is no file to show it shows the
//! options' empty text (the reference's "There is no preview for the
//! selected item").
//!
//! - Editable (Find in Files): the preview edits the file's document. When
//!   the file is open in a tab, the preview starts from the tab's text and
//!   every edit, in either view, goes to the other one too, so both show
//!   the same text and the tab saves it. A file that is not open gets a
//!   document of its own, saved like any edit (after the idle time, when
//!   the window loses the focus, when the preview shows another file and
//!   when the popup closes) without opening a tab; a tab that opens the
//!   file takes this document over.
//! - Read-only (Search Everywhere): a viewer ([`Editor::set_viewer`]) that
//!   takes the focus and shows a caret; typing shows "This view is
//!   read-only".
//!
//! The Soft-Wrap button (the reference's floating editor toolbar with
//! `EditorToggleUseSoftWrapsInPreview`) sits over the text's top right
//! corner: it fades in when the mouse moves over the preview and fades out
//! a while after it stopped (`TransparentComponentAnimator`). The choice is
//! one for all previews and lasts across sessions (the reference keeps it
//! for the `PREVIEW` place in the editor settings).

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use editor::{Editor, EditorEvent, SearchHighlights};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, EventEmitter, Global,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div, px,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use text::{AppliedEdit, Buffer, CaretPlacement, Edit, EditKind, Point};
use themes::UiRole;
use ui::components::{Tooltip, toggle_button};
use ui::icons::IconName;
use ui::theme::ThemeExt as _;

use crate::file_types::FileTypes;
use crate::workbench::Workbench;

ui::ui_commands! {
    COMMANDS, scope = "Editor";
    /// Soft wraps in the previews of Find in Files and Search Everywhere.
    ToggleSoftWraps => "preview.toggle-soft-wraps", "Soft-Wrap Preview Editor",
        "Turns soft wraps on or off in the previews of Find in Files and Search Everywhere (remembered).";
}

/// The empty text of a popup whose selected item has no file to show (the
/// reference's `usage.preview.isnt.available`).
pub const NO_PREVIEW: &str = "There is no preview for the selected item";
/// The Soft-Wrap button fades in this long, stays this long after the
/// mouse stopped over the preview, then fades out this long (the
/// reference's floating toolbar).
pub const SHOWING: Duration = Duration::from_millis(500);
pub const RETENTION: Duration = Duration::from_millis(1500);
pub const HIDING: Duration = Duration::from_millis(1000);
/// Distance of the Soft-Wrap button from the preview's top and right
/// edges (the floating toolbar's flow layout gaps).
const BUTTON_GAP: f32 = 20.0;

/// How a preview is set up.
#[derive(Debug, Clone)]
pub struct PreviewOptions {
    /// The text can be edited (Find in Files); else a read-only view.
    pub editable: bool,
    /// The Soft-Wrap button over the text.
    pub soft_wrap_button: bool,
    /// Shown by [`FilePreview::show_empty`].
    pub empty_text: SharedString,
    /// Prefix of the debug selectors (`<name>-soft-wrap`, `<name>-empty`).
    pub name: &'static str,
}

/// What the preview tells its owner.
#[derive(Debug, Clone)]
pub enum FilePreviewEvent {
    /// A file asked for with [`FilePreview::show_file`] is shown now: its
    /// owner marks the place to show.
    Loaded(PathBuf),
    /// The text changed (edits as applied, also those copied from the
    /// file's tab).
    Edited(Vec<AppliedEdit>),
    /// A message for the status bar (a save that failed).
    Message(String),
}

impl EventEmitter<FilePreviewEvent> for FilePreview {}

/// The Soft-Wrap button's opacity as it fades in and out.
#[derive(Debug, Clone, Copy)]
struct Fade {
    shown: bool,
    from: f32,
    at: Instant,
}

impl Fade {
    fn opacity(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.at).as_secs_f32();
        if self.shown {
            (self.from + t / SHOWING.as_secs_f32()).min(1.0)
        } else {
            (self.from - t / HIDING.as_secs_f32()).max(0.0)
        }
    }

    fn set(&mut self, shown: bool) {
        if self.shown != shown {
            let now = Instant::now();
            self.from = self.opacity(now);
            self.shown = shown;
            self.at = now;
        }
    }
}

pub struct FilePreview {
    options: PreviewOptions,
    editor: Entity<Editor>,
    file_types: Rc<FileTypes>,
    /// The main window, for the editors of open files and the save settings.
    host: Option<WeakEntity<Workbench>>,
    /// The file shown.
    path: Option<PathBuf>,
    /// The empty text is shown instead of the editor.
    empty: bool,
    /// The file being read.
    task: Option<Task<()>>,
    /// The editor of the file's tab: edits go both ways (editable only).
    linked: Option<(WeakEntity<Editor>, Subscription)>,
    /// The buffer versions an edit copied from the other view left in the
    /// preview and in the tab: their change events are not copied back.
    echo_preview: Option<u64>,
    echo_tab: Option<u64>,
    /// The idle save of the preview's own document.
    save_task: Option<Task<()>>,
    /// The Soft-Wrap button: shown, the mouse over it, and its hiding.
    fade: Fade,
    button_hovered: bool,
    hide_task: Option<Task<()>>,
    _subscriptions: [Subscription; 2],
}

impl FilePreview {
    pub fn new(
        options: PreviewOptions,
        file_types: Rc<FileTypes>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            let mut e = Editor::new(Buffer::new(""), window, cx);
            e.buffer_mut().set_read_only(true);
            e.set_viewer(!options.editable);
            e.set_soft_wraps(settings(cx).soft_wraps, cx);
            e
        });
        let subscriptions = [
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                if let EditorEvent::Edited(applied) = event {
                    this.edited(applied, cx);
                }
            }),
            // The choice made in one preview applies to all.
            cx.observe_global::<Store>(|this, cx| {
                let on = settings(cx).soft_wraps;
                this.editor.update(cx, |e, cx| e.set_soft_wraps(on, cx));
                cx.notify();
            }),
        ];
        Self {
            options,
            editor,
            file_types,
            host: None,
            path: None,
            empty: false,
            task: None,
            linked: None,
            echo_preview: None,
            echo_tab: None,
            save_task: None,
            fade: Fade {
                shown: false,
                from: 0.0,
                at: Instant::now(),
            },
            button_hovered: false,
            hide_task: None,
            _subscriptions: subscriptions,
        }
    }

    /// The main window the popup belongs to (its tabs' editors share
    /// their documents with the preview).
    pub fn set_host(&mut self, host: WeakEntity<Workbench>) {
        self.host = Some(host);
    }

    pub fn editor(&self) -> &Entity<Editor> {
        &self.editor
    }

    /// The file shown.
    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    /// Edits go to the file's tab and back.
    pub fn is_linked(&self) -> bool {
        self.linked.is_some()
    }

    /// The empty text shows.
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// The Soft-Wrap button shows (or fades in).
    pub fn soft_wrap_button_shown(&self) -> bool {
        self.options.soft_wrap_button && self.fade.shown
    }

    pub fn state_json(&self, rel: impl Fn(&Path) -> String, cx: &App) -> serde_json::Value {
        if self.empty {
            return json!({ "empty": self.options.empty_text.to_string() });
        }
        let Some(path) = &self.path else {
            return serde_json::Value::Null;
        };
        let e = self.editor.read(cx);
        json!({
            "path": rel(path),
            "caret": [e.caret_point().line, e.caret_point().column],
            "highlights": e.search_highlights().map(|h| h.matches.len()).unwrap_or(0),
            "read_only": e.buffer().is_read_only() || e.is_viewer(),
            "viewer": e.is_viewer(),
            "read_only_hint": e.read_only_hint(),
            "soft_wraps": e.soft_wraps(),
            "soft_wrap_button": self.soft_wrap_button_shown(),
            "linked": self.linked.is_some(),
            "modified": self.linked.is_none() && e.is_modified(),
        })
    }

    // ---- showing ------------------------------------------------------------

    /// Shows `path`: at once when it is shown already (`true`: the owner
    /// marks its place now), else once it is read
    /// ([`FilePreviewEvent::Loaded`]): the tab's text when the file is
    /// open, else the file read in the background. A file that cannot be
    /// read shows the empty text.
    pub fn show_file(&mut self, path: PathBuf, cx: &mut Context<Self>) -> bool {
        if self.path.as_ref() == Some(&path) && !self.empty {
            self.task = None;
            return true;
        }
        // Another file: the one shown so far is saved.
        self.save(cx);
        self.linked = None;
        if let Some(tab) = self.tab_editor(&path, cx) {
            let buffer = tab.read(cx).buffer().clone();
            self.load(path.clone(), buffer, cx);
            if self.options.editable {
                self.link(&tab, cx);
            }
            cx.emit(FilePreviewEvent::Loaded(path));
            return false;
        }
        let read = cx.background_spawn({
            let path = path.clone();
            async move {
                project::read_document(&path).map(|loaded| {
                    let read_only = loaded.read_only || loaded.decoded.lossy;
                    let mut buffer = Buffer::from_decoded(loaded.decoded);
                    buffer.set_read_only(read_only);
                    buffer
                })
            }
        });
        self.task = Some(cx.spawn(async move |this, cx| {
            let read = read.await;
            let _ = this.update(cx, |this, cx| {
                this.task = None;
                match read {
                    Ok(buffer) => {
                        this.load(path.clone(), buffer, cx);
                        cx.emit(FilePreviewEvent::Loaded(path));
                    }
                    Err(_) => this.show_empty(cx),
                }
            });
        }));
        false
    }

    /// The empty text instead of a file.
    pub fn show_empty(&mut self, cx: &mut Context<Self>) {
        self.clear(cx);
        self.empty = true;
        cx.notify();
    }

    /// Nothing shown; the preview's own document is saved first.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.save(cx);
        self.task = None;
        self.linked = None;
        self.empty = false;
        if self.path.take().is_some() {
            self.reset_editor(cx);
        }
        cx.notify();
    }

    fn reset_editor(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |e, cx| {
            *e.buffer_mut() = Buffer::new("");
            e.buffer_mut().set_read_only(true);
            e.set_path(None);
            e.buffer_reloaded(cx);
            e.set_search_highlights(None, cx);
        });
    }

    fn load(&mut self, path: PathBuf, buffer: Buffer, cx: &mut Context<Self>) {
        let file_type = self.file_types.for_path(&path).cloned();
        self.editor.update(cx, |e, cx| {
            *e.buffer_mut() = buffer;
            e.buffer_reloaded(cx);
            e.set_path(Some(path.clone()));
            match &file_type {
                Some(t) => {
                    e.set_config(t.editor_config());
                    e.set_highlighter(t.highlighter.as_ref().map(|f| f()), cx);
                }
                None => {
                    e.set_config(editor::EditorConfig::default());
                    e.set_highlighter(None, cx);
                }
            }
            e.set_search_highlights(None, cx);
        });
        self.path = Some(path);
        self.empty = false;
        cx.notify();
    }

    /// Marks `ranges` (char ranges of the text shown; `current` drawn as
    /// the current match, `line` as the found line); with `go`, the caret
    /// goes there, its line in the middle of the view.
    pub fn mark(
        &mut self,
        ranges: &[Range<usize>],
        current: Option<usize>,
        line: Option<usize>,
        go: Option<Point>,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |e, cx| {
            let mut matches: Vec<_> = ranges.iter().filter(|r| r.end > r.start).cloned().collect();
            matches.sort_by_key(|r| r.start);
            let current = current.filter(|&i| i < matches.len());
            e.set_search_highlights(
                Some(SearchHighlights {
                    matches,
                    current,
                    current_background: false,
                    excluded: Default::default(),
                    lines: line.map(|l| vec![l]).unwrap_or_default(),
                }),
                cx,
            );
            if let Some(p) = go {
                e.go_to(p.line + 1, Some(p.column + 1), cx);
            }
        });
        cx.notify();
    }

    // ---- the file's tab and saving ------------------------------------------

    /// The editor of the tab of `path`, when the file is open.
    fn tab_editor(&self, path: &Path, cx: &App) -> Option<Entity<Editor>> {
        let host = self.host.as_ref()?.upgrade()?;
        host.read(cx).editor_for(path)
    }

    /// The preview's editor when it edits `path`'s own document (the file
    /// is not open in a tab).
    pub fn own_editor(&self, path: &Path) -> Option<Entity<Editor>> {
        (self.options.editable && self.path.as_deref() == Some(path) && self.linked.is_none())
            .then(|| self.editor.clone())
    }

    /// Edits of the tab's editor go to the preview from now on (and the
    /// preview's to the tab).
    fn link(&mut self, tab: &Entity<Editor>, cx: &mut Context<Self>) {
        let subscription = cx.subscribe(tab, |this, tab, event: &EditorEvent, cx| {
            let EditorEvent::Edited(applied) = event else {
                return;
            };
            let version = tab.read(cx).buffer().version();
            if this.echo_tab.take() == Some(version) {
                return;
            }
            let preview = this.editor.clone();
            mirror(applied, &tab, &preview, cx);
            this.echo_preview = Some(preview.read(cx).buffer().version());
        });
        self.linked = Some((tab.downgrade(), subscription));
    }

    /// The preview's text changed: the owner is told, and the edit goes to
    /// the tab, or the preview's own document is saved after the idle time.
    fn edited(&mut self, applied: &[AppliedEdit], cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        // Also for an edit copied from the tab.
        cx.emit(FilePreviewEvent::Edited(applied.to_vec()));
        let version = self.editor.read(cx).buffer().version();
        if self.echo_preview.take() == Some(version) {
            return;
        }
        // Still the tab's editor? (The tab may have been closed.)
        let linked = self
            .linked
            .as_ref()
            .and_then(|(tab, _)| tab.upgrade())
            .filter(|tab| self.tab_editor(&path, cx).as_ref() == Some(tab));
        match linked {
            Some(tab) => {
                let preview = self.editor.clone();
                mirror(applied, &preview, &tab, cx);
                self.echo_tab = Some(tab.read(cx).buffer().version());
            }
            None => {
                self.linked = None;
                self.schedule_save(cx);
            }
        }
    }

    /// Saves the preview's own document once there was no input for the
    /// idle time (the workbench's setting).
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        let settings = self
            .host
            .as_ref()
            .and_then(|h| h.upgrade())
            .map(|h| h.read(cx).save_settings())
            .unwrap_or_default();
        if !settings.save_on_idle {
            return;
        }
        let idle = settings.idle();
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(idle).await;
            let _ = this.update(cx, |this, cx| this.save(cx));
        }));
    }

    /// Writes the preview's own document when it has unsaved edits (a
    /// file open in a tab is saved by its tab).
    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.linked.is_some() || !self.editor.read(cx).is_modified() {
            return;
        }
        let editor = self.editor.clone();
        let (bytes, point) = editor.update(cx, |e, _| {
            let point = e.buffer_mut().begin_save();
            (e.buffer().encoded(), point)
        });
        let write = cx.background_spawn({
            let path = path.clone();
            async move { project::write_document(&path, &bytes) }
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |_, cx| match result {
                Ok(()) => editor.update(cx, |e, cx| {
                    e.buffer_mut().mark_saved_at(point);
                    cx.notify();
                }),
                Err(e) => cx.emit(FilePreviewEvent::Message(format!(
                    "Cannot save {}: {e}",
                    path.display()
                ))),
            });
        })
        .detach();
    }

    /// The preview's own document of `path`, for a tab that opens the file
    /// (the tab continues with its text and history).
    pub fn document(&self, path: &Path, cx: &App) -> Option<Buffer> {
        self.own_editor(path).map(|e| e.read(cx).buffer().clone())
    }

    /// [`Self::document`], and the preview lets go of it (it is not saved
    /// by the preview any more).
    pub fn take_document(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<Buffer> {
        let document = self.document(path, cx)?;
        self.save_task = None;
        self.path = None;
        self.reset_editor(cx);
        Some(document)
    }

    /// A tab opened `path`: when the preview edits it, the two views are
    /// linked (they hold the same text).
    pub fn tab_opened(&mut self, path: &Path, tab: &Entity<Editor>, cx: &mut Context<Self>) {
        if self.own_editor(path).is_some() {
            self.save_task = None;
            self.link(tab, cx);
        }
    }

    // ---- the Soft-Wrap button -------------------------------------------------

    /// The mouse moved over the preview: the button fades in and stays a
    /// while.
    fn mouse_moved(&mut self, cx: &mut Context<Self>) {
        if !self.options.soft_wrap_button {
            return;
        }
        let was_shown = self.fade.shown;
        self.fade.set(true);
        let wait = if was_shown {
            RETENTION
        } else {
            SHOWING + RETENTION
        };
        self.hide_later(wait, cx);
        if !was_shown {
            cx.notify();
        }
    }

    fn hide_later(&mut self, wait: Duration, cx: &mut Context<Self>) {
        self.hide_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.hide_task = None;
                // Kept while the mouse is over it.
                if !this.button_hovered {
                    this.fade.set(false);
                    cx.notify();
                }
            });
        }));
    }

    fn button_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.button_hovered = hovered;
        if !hovered && self.fade.shown && self.hide_task.is_none() {
            self.hide_later(RETENTION, cx);
        }
    }

    fn render_button(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.options.soft_wrap_button || self.empty {
            return None;
        }
        let opacity = self.fade.opacity(Instant::now());
        let target = if self.fade.shown { 1.0 } else { 0.0 };
        if opacity != target {
            window.request_animation_frame();
        }
        if opacity <= 0.0 && !self.fade.shown {
            return None;
        }
        let name = self.options.name;
        let on = self.editor.read(cx).soft_wraps();
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .top(px(BUTTON_GAP))
                .right(px(BUTTON_GAP))
                // Over the editor: it takes the mouse from the editor, whose
                // text cursor must not show through (ui::pointer); the wheel
                // still scrolls the editor.
                .block_mouse_except_scroll()
                .opacity(opacity)
                .p(px(2.0))
                .rounded(px(3.0))
                .bg(theme.color(UiRole::SurfaceRaised))
                .border_1()
                .border_color(theme.color(UiRole::PopupBorder))
                .child(
                    toggle_button(
                        SharedString::from(format!("{name}-soft-wrap")),
                        IconName::SOFT_WRAP,
                        on,
                        cx,
                    )
                    .debug_selector(move || format!("{name}-soft-wrap"))
                    .tooltip(Tooltip::text("Soft-Wrap Preview Editor", None))
                    .on_hover(
                        cx.listener(|this, hovered: &bool, _, cx| this.button_hover(*hovered, cx)),
                    )
                    // The focus stays where it is.
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(|_: &ClickEvent, _, cx| toggle_soft_wraps(cx)),
                )
                .into_any_element(),
        )
    }
}

impl Render for FilePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.options.name;
        let button = self.render_button(window, cx);
        let theme = cx.theme();
        let content = if self.empty {
            div()
                .id(SharedString::from(format!("{name}-empty")))
                .debug_selector(move || format!("{name}-empty"))
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(self.options.empty_text.clone())
                .into_any_element()
        } else {
            self.editor.clone().into_any_element()
        };
        div()
            .id(SharedString::from(format!("{name}-area")))
            .debug_selector(move || format!("{name}-area"))
            .relative()
            .size_full()
            .on_mouse_move(cx.listener(|this, _, _, cx| this.mouse_moved(cx)))
            .child(content)
            .children(button)
    }
}

// ---- the Soft-Wrap choice -----------------------------------------------------

/// The previews' settings that last across sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PreviewSettings {
    /// Soft wraps in the previews.
    pub soft_wraps: bool,
}

#[derive(Default)]
struct Store {
    settings: PreviewSettings,
    file: Option<PathBuf>,
}

impl Global for Store {}

/// Where the settings are kept: next to Find in Files' (`PYOXIDE_CONFIG_DIR`,
/// else the user's configuration folder), in `preview.toml`.
pub fn settings_file() -> Option<PathBuf> {
    crate::find_in_files::settings_file().map(|f| f.with_file_name("preview.toml"))
}

/// Reads the settings from `file` and saves them there from now on.
pub fn load_settings(cx: &mut App, file: PathBuf) {
    let loaded = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| toml::from_str::<PreviewSettings>(&text).ok());
    let store = cx.default_global::<Store>();
    if let Some(settings) = loaded {
        store.settings = settings;
    }
    store.file = Some(file);
}

pub fn settings(cx: &App) -> PreviewSettings {
    cx.try_global::<Store>()
        .map(|s| s.settings.clone())
        .unwrap_or_default()
}

/// Turns soft wraps in the previews on or off, and saves the choice.
pub fn toggle_soft_wraps(cx: &mut App) {
    let store = cx.default_global::<Store>();
    store.settings.soft_wraps = !store.settings.soft_wraps;
    let (Some(file), Ok(text)) = (store.file.clone(), toml::to_string(&store.settings)) else {
        return;
    };
    cx.background_spawn(async move {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&file, text) {
            eprintln!("cannot save {}: {e}", file.display());
        }
    })
    .detach();
}

/// Registers the commands.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ToggleSoftWraps, cx| toggle_soft_wraps(cx));
}

// ---- copying edits between the preview and the tab ---------------------------

/// Copies an edit of `from` (as applied) to `to`, which held the same text
/// before it; `to` keeps its carets where its text kept them.
fn mirror(applied: &[AppliedEdit], from: &Entity<Editor>, to: &Entity<Editor>, cx: &mut App) {
    let edits = replayed(applied, from.read(cx).buffer());
    let delta: isize = edits
        .iter()
        .map(|e| e.text.chars().count() as isize - e.range.len() as isize)
        .sum();
    let before = (from.read(cx).buffer().len_chars() as isize - delta) as usize;
    if to.read(cx).buffer().len_chars() != before {
        // The views went apart (not expected): the target takes the text.
        let text = from.read(cx).text();
        to.update(cx, |e, cx| {
            let len = e.buffer().len_chars();
            e.edit(
                vec![Edit::new(0..len, text)],
                EditKind::Other,
                CaretPlacement::AfterEachEdit,
                cx,
            )
        });
        return;
    }
    let typing = edits.len() == 1 && edits[0].range.is_empty() && !edits[0].text.contains('\n');
    let kind = if typing {
        EditKind::Typing
    } else {
        EditKind::Other
    };
    let ranges: Vec<(Range<usize>, usize)> = edits
        .iter()
        .map(|e| (e.range.clone(), e.text.chars().count()))
        .collect();
    to.update(cx, |e, cx| {
        let mut carets = e.selections().clone();
        carets.update(|s| {
            s.anchor = map_offset(s.anchor, &ranges);
            s.head = map_offset(s.head, &ranges);
        });
        e.edit(edits, kind, CaretPlacement::Explicit(carets), cx)
    });
}

/// The edits that turn the text before an edit into the text after it, in
/// the coordinates before it, from the edit as applied (descending
/// positions, each applied in turn) and the text after it.
fn replayed(applied: &[AppliedEdit], after: &Buffer) -> Vec<Edit> {
    let mut shift: isize = 0;
    let mut edits = Vec::with_capacity(applied.len());
    for a in applied.iter().rev() {
        let start = (a.start as isize + shift) as usize;
        let end = (a.new_end as isize + shift) as usize;
        edits.push(Edit::new(a.start..a.old_end, after.slice(start..end)));
        shift += a.new_end as isize - a.old_end as isize;
    }
    edits.sort_by_key(|e| e.range.start);
    edits
}

/// Where a position of the text before edits (ranges and new lengths,
/// ascending) is after them; a position inside a replaced range goes to
/// the end of its replacement.
fn map_offset(pos: usize, edits: &[(Range<usize>, usize)]) -> usize {
    let mut delta: isize = 0;
    for (range, new_len) in edits {
        if pos < range.start {
            break;
        }
        if range.is_empty() || pos >= range.end {
            // Before the position (an insertion at it goes before it too).
            delta += *new_len as isize - range.len() as isize;
        } else {
            return (range.start as isize + delta) as usize + new_len;
        }
    }
    (pos as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use text::SelectionSet;

    use super::*;

    #[test]
    fn edits_are_replayed_in_the_text_before_them() {
        let mut a = Buffer::new("one two three four");
        let mut sel = SelectionSet::caret(0);
        let applied = a.transact(
            &mut sel,
            vec![Edit::new(0..3, "1"), Edit::new(8..13, "THREE!")],
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        let edits = replayed(&applied, &a);
        let mut b = Buffer::new("one two three four");
        let mut sel = SelectionSet::caret(0);
        b.transact(
            &mut sel,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), a.text());
        assert_eq!(b.text(), "1 two THREE! four");
        // An undo is replayed the same way.
        let (undone, _) = a.undo().unwrap();
        let edits = replayed(&undone, &a);
        b.transact(
            &mut sel,
            edits,
            EditKind::Other,
            CaretPlacement::AfterEachEdit,
        );
        assert_eq!(b.text(), "one two three four");
    }

    #[test]
    fn carets_follow_the_edits() {
        let edits = [(2..4, 5), (10..10, 3)];
        assert_eq!(map_offset(1, &edits), 1);
        assert_eq!(map_offset(3, &edits), 7, "inside: the end of the new text");
        assert_eq!(map_offset(6, &edits), 9);
        assert_eq!(map_offset(10, &edits), 16, "at an insertion: after it");
        assert_eq!(map_offset(12, &edits), 18);
    }

    #[test]
    fn the_button_fades_in_and_out() {
        let at = Instant::now();
        let mut fade = Fade {
            shown: true,
            from: 0.0,
            at,
        };
        assert_eq!(fade.opacity(at), 0.0);
        assert!((fade.opacity(at + SHOWING / 2) - 0.5).abs() < 0.01);
        assert_eq!(fade.opacity(at + SHOWING), 1.0);
        fade = Fade {
            shown: false,
            from: 1.0,
            at,
        };
        assert!((fade.opacity(at + HIDING / 2) - 0.5).abs() < 0.01);
        assert_eq!(fade.opacity(at + HIDING * 2), 0.0);
    }

    #[test]
    fn the_soft_wrap_choice_is_read_back() {
        let parsed: PreviewSettings = toml::from_str("soft_wraps = true\n").unwrap();
        assert!(parsed.soft_wraps);
        assert_eq!(
            toml::from_str::<PreviewSettings>("").unwrap(),
            PreviewSettings::default()
        );
    }
}
