//! Editor > Font (`AppEditorFontConfigurable`: `AppFontPanel` with
//! `AppEditorFontOptionsPanel` and `FontEditorPreview`): the editor font
//! (`[editor.font]`), applied over the theme's to every editor and to the
//! Run console when the page is applied.
//!
//! The options on the left: Font, Size and Line height on one row, Enable
//! ligatures with its help, the comment pointing to Reader mode, "Restore
//! defaults" while the font differs from the defaults, and the collapsible
//! Typography Settings with the fallback font. On the right an editable
//! preview shows the font the page holds. The families come from the text
//! system, listed and probed (monospaced, able to show its own name) on the
//! background executor, monospaced ones first, as the reference's
//! `FontFamilyCombo`.
//!
//! Not drawn: the main and bold weights, the character variants, the
//! whitespace dots of the preview and its blinking of changed glyphs.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use editor::{Editor, EditorEvent, Highlighter, InlineStyle, Span};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, ElementId, Entity, FocusHandle,
    FontWeight, Global, IntoElement, KeyDownEvent, Pixels, Render, ScrollStrategy, SharedString,
    Subscription, Task, TextSystem, UniformListScrollHandle, Window, anchored, canvas, div,
    prelude::*, px, uniform_list,
};
use serde_json::{Value as Json, json};
use settings::{EditorFontSettings, Section as _};
use text::{AppliedEdit, Buffer};
use themes::UiRole;
use ui::components::Tooltip;
use ui::deferred;
use ui::icons::{Icon, IconName};
use ui::pointer::PointerExt as _;
use ui::theme::{FontChoice, Theme, ThemeExt};

use crate::page::{PageContext, SettingsPage};
use crate::store;

pub const ID: &str = "editor.font";
pub const TITLE: &str = "Font";

/// The reference's labels (ApplicationBundle).
const FONT: &str = "Font:";
const SIZE: &str = "Size:";
const LINE_HEIGHT: &str = "Line height:";
const LIGATURES: &str = "Enable ligatures";
const LIGATURES_TOOLTIP: &str = "Use several characters as a single ligature. Ligatures enhance code readability. Use the section below to preview the changes";
const RESTORE: &str = "Restore defaults";
/// The comment under the ligatures (`comment.use.ligatures.with.reader.
/// mode`): the text and its link to the Reader mode page.
const READER_COMMENT: &str = "See line height and ligatures also in ";
const READER_LINK: &str = "Reader mode";
/// The Reader mode settings page (`ReaderModeConfigurable`); the link
/// selects it when it is registered.
const READER_PAGE: &str = "editor.reader.mode";
const TYPOGRAPHY: &str = "Typography Settings";
const FALLBACK: &str = "Fallback font:";
const FALLBACK_COMMENT: &str = "Used for symbols not supported\nby the main font";
const NONE: &str = "<None>";
const MONOSPACED: &str = "Monospaced";
const PROPORTIONAL: &str = "Proportional";
const PREVIEW_HINT: &str = "Enter any text to preview";
pub const SEARCH_TERMS: &[&str] = &[
    FONT,
    SIZE,
    LINE_HEIGHT,
    LIGATURES,
    RESTORE,
    TYPOGRAPHY,
    FALLBACK,
    FALLBACK_COMMENT,
];

/// The preview's text (`FontPreviewService.fontPreviewText`, with the
/// product's name): `<bold>` marks the parts shown in bold.
const PREVIEW_TEXT: &str = "Pyoxide is an <bold>Integrated \n\
Development Environment (IDE)</bold> designed\n\
to maximize productivity. It provides\n\
<bold>clever code completion, static code\n\
analysis, and refactorings,</bold> and lets\n\
you focus on the bright side of\n\
software development making\n\
it an enjoyable experience.\n\
\n\
Default:\n\
abcdefghijklmnopqrstuvwxyz\n\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\n\
0123456789 (){}[]\n\
+ - * / = .,;:!? #&$%@|^\n\
\n\
<bold>Bold:\n\
abcdefghijklmnopqrstuvwxyz\n\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\n\
0123456789 (){}[]\n\
+ - * / = .,;:!? #&$%@|^</bold>\n\
\n\
<!-- -- != := === >= >- >=> |-> -> <$>\n\
</> #[ |||> |= ~@\n\
\n\
\n";

/// Families the reference counts as monospaced before probing
/// (`FontFamilyCombo.MyModel.KNOWN_MONOSPACED_FAMILIES`).
const KNOWN_MONOSPACED: &[&str] = &[
    "Consolas",
    "DejaVu Sans Mono",
    "Droid Sans Mono",
    "JetBrains Mono",
    "Fira Code",
    "Inconsolata",
    "Menlo",
    "Monospaced",
    "Source Code Pro",
];

/// The page's metrics, from the reference (Kotlin UI DSL, `AppFontPanel`):
/// the options 12 from the page's edge and the top line, rows 24 high and
/// 12 apart, the label column 37 wide, the font combo 285 wide and the
/// fallback combo 270, the size fields 43, 17 between Size's field and Line
/// height, groups indented by 20, the preview's hint 10 from the text with
/// the small font. The rows after Size (the ligatures, the Reader mode
/// comment, Restore defaults, Typography Settings) are as far apart as the
/// reference draws them (centre to centre about 41, 37, 26 and 51 from
/// Size on): Size's bottom gap and the check box's insets before the
/// ligatures, the comment's top gap after them, the Typography group's top
/// gap before it.
const PAD: f32 = 12.0;
const ROW: f32 = 24.0;
const ROW_GAP: f32 = 12.0;
const SMALL_GAP: f32 = 8.0;
const LIGATURES_TOP: f32 = 17.0;
const READER_TOP: f32 = 15.0;
const RESTORE_TOP: f32 = 7.0;
const LABEL_COLUMN: f32 = 37.0;
const LABEL_GAP: f32 = 6.0;
const FAMILY_WIDTH: f32 = 285.0;
const FALLBACK_WIDTH: f32 = 270.0;
const NUMBER_WIDTH: f32 = 43.0;
const SIZE_GAP: f32 = 17.0;
const HELP_GAP: f32 = 6.0;
const GROUP_TOP: f32 = 26.0;
const GROUP_INDENT: f32 = 20.0;
const COMMENT_TOP: f32 = 4.0;
const PREVIEW_LEFT: f32 = 6.0;
const HINT_PAD: (f32, f32) = (10.0, 15.0);
/// The list of a combo box: rows of `List.rowHeight`, at most this many
/// shown, the body's insets.
const LIST_ROW: f32 = 24.0;
const LIST_ROWS: usize = 15;
const LIST_PAD: f32 = 8.0;

/// The keys of `settings.edit`.
mod field {
    pub const FAMILY: &str = "family";
    pub const SIZE: &str = "size";
    pub const LINE_HEIGHT: &str = "line-height";
    pub const LIGATURES: &str = "ligatures";
    pub const FALLBACK: &str = "fallback";
    pub const RESTORE: &str = "restore-defaults";
    pub const TYPOGRAPHY: &str = "typography";
    pub const LIST: &str = "list";
    pub const CHOOSE: &str = "choose";
}

// ---- the store and the theme -------------------------------------------------------

/// Registers `[editor.font]` and puts the stored font over the theme's
/// from now on (start-up, and every change of the section).
pub fn init(cx: &mut App) {
    store::register::<EditorFontSettings>(cx);
    store::observe(cx, EditorFontSettings::KEY, apply_stored);
    apply_stored(cx);
}

/// The theme's choice for stored settings (sizes kept in range).
pub fn choice_of(settings: &EditorFontSettings) -> FontChoice {
    FontChoice {
        family: settings.family.clone(),
        size: settings.size.map(EditorFontSettings::clamp_size),
        line_spacing: settings
            .line_height
            .map(EditorFontSettings::clamp_line_height),
        ligatures: settings.ligatures,
        fallback: settings.fallback.clone(),
    }
}

fn apply_stored(cx: &mut App) {
    let settings: EditorFontSettings = store::get(cx);
    ui::theme::set_font_choice(cx, choice_of(&settings));
}

// ---- font families -------------------------------------------------------------------

/// A font family of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Family {
    pub name: SharedString,
    pub monospaced: bool,
    /// The font has glyphs for its own name (the list draws it in it; the
    /// main font's list leaves it out otherwise).
    pub shows_name: bool,
}

/// The families known so far and the listing in progress.
#[derive(Default)]
struct Families {
    list: Arc<Vec<Family>>,
    loaded: bool,
    task: Option<Task<()>>,
}

impl Global for Families {}

/// Lists and probes the families of the text system (blocking: every
/// family is loaded; run it off the UI thread).
fn probe_families(ts: &TextSystem) -> Vec<Family> {
    let mut names: Vec<String> = ts
        .all_font_names()
        .into_iter()
        .filter(|n| !n.is_empty() && !n.starts_with('.'))
        .collect();
    names.sort();
    names.dedup();
    let size = px(12.0);
    let mut families: Vec<Family> = names
        .into_iter()
        .map(|name| {
            let id = ts.resolve_font(&gpui::font(name.clone()));
            let width = |c: char| ts.advance(id, size, c).ok().map(|s| s.width);
            let widths = [width('i'), width('m'), width('W'), width('.')];
            let monospaced = KNOWN_MONOSPACED.contains(&name.as_str())
                || (widths[0].is_some() && widths.iter().all(|w| *w == widths[0]));
            let shows_name = name.chars().all(|c| c == ' ' || width(c).is_some());
            Family {
                name: name.into(),
                monospaced,
                shows_name,
            }
        })
        .collect();
    sort_families(&mut families);
    families
}

/// Monospaced families first, then by name (`MyFontItemComparator`).
fn sort_families(families: &mut [Family]) {
    families.sort_by(|a, b| {
        b.monospaced
            .cmp(&a.monospaced)
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Lists the families on the background executor, once per application;
/// the windows redraw when they are there.
fn refresh_families(cx: &mut App) {
    if cx
        .try_global::<Families>()
        .is_some_and(|f| f.loaded || f.task.is_some())
    {
        return;
    }
    let ts = cx.text_system().clone();
    let work = cx.background_spawn(async move { probe_families(&ts) });
    let task = cx.spawn(async move |cx| {
        let list = work.await;
        let _ = cx.update(|cx| {
            let families = cx.default_global::<Families>();
            families.list = Arc::new(list);
            families.loaded = true;
            families.task = None;
            cx.refresh_windows();
        });
    });
    cx.default_global::<Families>().task = Some(task);
}

fn families(cx: &App) -> (Arc<Vec<Family>>, bool) {
    cx.try_global::<Families>()
        .map(|f| (f.list.clone(), f.loaded))
        .unwrap_or_default()
}

// ---- the preview ---------------------------------------------------------------------

/// The preview's text without the marks, and the ranges in bold.
fn preview_text() -> (String, Vec<Range<usize>>) {
    const OPEN: &str = "<bold>";
    const CLOSE: &str = "</bold>";
    let mut text = String::new();
    let mut bold = Vec::new();
    let mut rest = PREVIEW_TEXT;
    while let Some(start) = rest.find(OPEN) {
        text.push_str(&rest[..start]);
        let from = text.chars().count();
        rest = &rest[start + OPEN.len()..];
        let end = rest.find(CLOSE).unwrap_or(rest.len());
        text.push_str(&rest[..end]);
        bold.push(from..text.chars().count());
        rest = rest.get(end + CLOSE.len()..).unwrap_or("");
    }
    text.push_str(rest);
    (text, bold)
}

/// Draws the preview in the editor's text colour with the bold parts in
/// bold; the parts follow the edits of the text.
struct PreviewHighlighter {
    /// Char ranges of the text.
    bold: Vec<Range<usize>>,
    /// Char ranges of each line with their weight.
    lines: Vec<Vec<(Range<usize>, bool)>>,
}

impl PreviewHighlighter {
    fn split(&mut self, buffer: &Buffer) {
        self.lines = (0..buffer.len_lines())
            .map(|line| {
                let start = buffer.line_start(line);
                let end = start + buffer.line_len(line);
                let mut parts = Vec::new();
                let mut at = start;
                for range in &self.bold {
                    let (s, e) = (range.start.max(start), range.end.min(end));
                    if s >= e {
                        continue;
                    }
                    if at < s {
                        parts.push((at - start..s - start, false));
                    }
                    parts.push((s - start..e - start, true));
                    at = e;
                }
                if at < end {
                    parts.push((at - start..end - start, false));
                }
                parts
            })
            .collect();
    }
}

impl Highlighter for PreviewHighlighter {
    fn reset(&mut self, buffer: &Buffer) {
        self.split(buffer);
    }

    fn edited(&mut self, buffer: &Buffer, edits: &[AppliedEdit]) {
        for edit in edits {
            let moved = |at: usize| {
                if at >= edit.old_end {
                    at + edit.new_end - edit.old_end
                } else {
                    at.min(edit.start)
                }
            };
            for range in &mut self.bold {
                *range = moved(range.start)..moved(range.end);
            }
            self.bold.retain(|r| r.start < r.end);
        }
        self.split(buffer);
    }

    fn line_spans(&self, _: usize, _: &mut Vec<Span>) {}

    fn line_styles(&self, line: usize, theme: &Theme, out: &mut Vec<(Range<usize>, InlineStyle)>) {
        let color = Some(theme.color(UiRole::EditorText));
        for (range, bold) in self.lines.get(line).into_iter().flatten() {
            out.push((
                range.clone(),
                InlineStyle {
                    color,
                    bold: *bold,
                    ..InlineStyle::default()
                },
            ));
        }
    }
}

// ---- the page ------------------------------------------------------------------------

/// Which combo box's list is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    Family,
    Fallback,
}

impl List {
    fn name(self) -> &'static str {
        match self {
            List::Family => "family",
            List::Fallback => "fallback",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A row of a combo box's list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(&'static str),
    None,
    Family(Family),
}

impl Row {
    fn value(&self) -> Option<Option<SharedString>> {
        match self {
            Row::Header(_) => None,
            Row::None => Some(None),
            Row::Family(f) => Some(Some(f.name.clone())),
        }
    }
}

/// An open list: its rows, the highlighted one and its scroll.
struct OpenList {
    which: List,
    rows: Rc<Vec<Row>>,
    highlighted: Option<usize>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

/// The font the page holds, in the form the store keeps it.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    family: String,
    size: f32,
    line_height: f32,
    ligatures: bool,
    fallback: Option<String>,
}

impl Shown {
    fn defaults() -> Self {
        Self {
            family: EditorFontSettings::DEFAULT_FAMILY.to_owned(),
            size: EditorFontSettings::DEFAULT_SIZE,
            line_height: EditorFontSettings::DEFAULT_LINE_HEIGHT,
            ligatures: false,
            fallback: None,
        }
    }

    /// What is stored, the theme's own font for the rest.
    fn stored(cx: &App) -> Self {
        let stored: EditorFontSettings = store::get(cx);
        let own = cx.theme().theme_editor_font();
        Self {
            family: stored.family.unwrap_or_else(|| own.family.to_string()),
            size: stored
                .size
                .map(EditorFontSettings::clamp_size)
                .unwrap_or(f32::from(own.size)),
            line_height: stored
                .line_height
                .map(EditorFontSettings::clamp_line_height)
                .unwrap_or(own.line_spacing),
            ligatures: stored.ligatures.unwrap_or(own.ligatures),
            fallback: stored.fallback,
        }
    }

    /// The settings to store: only what differs from the theme's font.
    fn settings(&self, cx: &App) -> EditorFontSettings {
        let own = cx.theme().theme_editor_font();
        EditorFontSettings {
            family: (self.family != own.family.as_ref()).then(|| self.family.clone()),
            size: (self.size != f32::from(own.size)).then_some(self.size),
            line_height: (self.line_height != own.line_spacing).then_some(self.line_height),
            ligatures: (self.ligatures != own.ligatures).then_some(self.ligatures),
            fallback: self.fallback.clone(),
        }
    }

    fn choice(&self) -> FontChoice {
        FontChoice {
            family: Some(self.family.clone()),
            size: Some(self.size),
            line_spacing: Some(self.line_height),
            ligatures: Some(self.ligatures),
            fallback: self.fallback.clone(),
        }
    }
}

/// A number as the reference's fields show it (`Float.toString`: `14.0`,
/// `1.2`).
fn number_text(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

/// The number of a field, kept in `range` (`MathUtil.clamp`); an error
/// message for text that is no number.
fn parse_number(text: &str, range: &std::ops::RangeInclusive<f32>) -> Result<f32, String> {
    text.trim()
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(*range.start(), *range.end()))
        .ok_or_else(|| {
            format!(
                "Please enter a number from {} to {}",
                range.start(),
                range.end()
            )
        })
}

/// The page: the font it holds (Size and Line height as typed), the
/// preview, an open list.
pub struct FontPage {
    family: String,
    fallback: Option<String>,
    ligatures: bool,
    size: Entity<Editor>,
    line_height: Entity<Editor>,
    typography: bool,
    list: Option<OpenList>,
    /// Where the combo boxes are drawn (the lists open below them).
    anchors: Rc<RefCell<[Bounds<Pixels>; 2]>>,
    preview: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FontPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        refresh_families(cx);
        let shown = Shown::stored(cx);
        let size = editor::field::new(&number_text(shown.size), None, true, window, cx);
        let line_height =
            editor::field::new(&number_text(shown.line_height), None, true, window, cx);
        let (text, bold) = preview_text();
        let preview = cx.new(|cx| {
            let mut editor = Editor::new(Buffer::new(&text), window, cx);
            editor.set_console(true);
            editor.set_transparent(true);
            editor.set_highlighter(
                Some(Box::new(PreviewHighlighter {
                    bold,
                    lines: Vec::new(),
                })),
                cx,
            );
            editor
        });
        let edited =
            |this: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
                if matches!(e, EditorEvent::Edited(_)) {
                    this.sync_preview(cx);
                    cx.notify();
                }
            };
        let subscriptions = vec![
            cx.subscribe(&size, edited),
            cx.subscribe(&line_height, edited),
        ];
        let mut this = Self {
            family: shown.family,
            fallback: shown.fallback,
            ligatures: shown.ligatures,
            size,
            line_height,
            typography: false,
            list: None,
            anchors: Rc::default(),
            preview,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.sync_preview(cx);
        this
    }

    fn text(field: &Entity<Editor>, cx: &App) -> String {
        field.read(cx).text().trim().to_owned()
    }

    /// The font the page holds; an error while Size or Line height is no
    /// number.
    fn shown(&self, cx: &App) -> Result<Shown, String> {
        Ok(Shown {
            family: self.family.clone(),
            size: parse_number(&Self::text(&self.size, cx), &EditorFontSettings::SIZES)?,
            line_height: parse_number(
                &Self::text(&self.line_height, cx),
                &EditorFontSettings::LINE_HEIGHTS,
            )?,
            ligatures: self.ligatures,
            fallback: self.fallback.clone(),
        })
    }

    /// The font the preview shows: the page's, with the stored values for
    /// a field that holds no number.
    fn preview_shown(&self, cx: &App) -> Shown {
        let stored = Shown::stored(cx);
        let number = |field: &Entity<Editor>, range, fallback: f32| {
            parse_number(&Self::text(field, cx), range).unwrap_or(fallback)
        };
        Shown {
            family: self.family.clone(),
            size: number(&self.size, &EditorFontSettings::SIZES, stored.size),
            line_height: number(
                &self.line_height,
                &EditorFontSettings::LINE_HEIGHTS,
                stored.line_height,
            ),
            ligatures: self.ligatures,
            fallback: self.fallback.clone(),
        }
    }

    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        let font = cx
            .theme()
            .editor_font_with(&self.preview_shown(cx).choice());
        self.preview.update(cx, |e, cx| e.set_font(Some(font), cx));
    }

    /// The page holds another font than the defaults (Restore defaults is
    /// enabled).
    fn restorable(&self, cx: &App) -> bool {
        self.shown(cx).map_or(true, |s| s != Shown::defaults())
    }

    fn set_numbers(&mut self, size: f32, line_height: f32, cx: &mut Context<Self>) {
        let size = number_text(size);
        let line_height = number_text(line_height);
        self.size.update(cx, |e, cx| e.set_text_selected(&size, cx));
        self.line_height
            .update(cx, |e, cx| e.set_text_selected(&line_height, cx));
    }

    /// Restore defaults (`AppFontOptions.initDefaults`): JetBrains Mono at
    /// 13 with line height 1.2, no ligatures, no fallback font.
    fn restore_defaults(&mut self, cx: &mut Context<Self>) {
        let defaults = Shown::defaults();
        self.family = defaults.family;
        self.ligatures = defaults.ligatures;
        self.fallback = defaults.fallback;
        self.set_numbers(defaults.size, defaults.line_height, cx);
        self.sync_preview(cx);
        cx.notify();
    }

    fn toggle_ligatures(&mut self, cx: &mut Context<Self>) {
        self.ligatures = !self.ligatures;
        self.sync_preview(cx);
        cx.notify();
    }

    /// Up and Down in Size (by 1) and Line height (by 0.1), kept in range.
    fn step(&mut self, line_height: bool, up: bool, cx: &mut Context<Self>) {
        let (field, range, step) = if line_height {
            (&self.line_height, EditorFontSettings::LINE_HEIGHTS, 0.1)
        } else {
            (&self.size, EditorFontSettings::SIZES, 1.0)
        };
        let Ok(value) = Self::text(field, cx).parse::<f32>() else {
            return;
        };
        let value = (value + if up { step } else { -step }).clamp(*range.start(), *range.end());
        let text = if line_height {
            format!("{value:.1}")
        } else {
            number_text(value)
        };
        field
            .clone()
            .update(cx, |e, cx| e.set_text_selected(&text, cx));
    }

    // ---- lists ----

    fn rows(which: List, cx: &App) -> Vec<Row> {
        let (list, _) = families(cx);
        let mut rows = Vec::new();
        if which == List::Fallback {
            rows.push(Row::None);
        }
        let mut monospaced = None;
        for family in list.iter() {
            if which == List::Family && !family.shows_name {
                continue;
            }
            if monospaced != Some(family.monospaced) {
                monospaced = Some(family.monospaced);
                rows.push(Row::Header(if family.monospaced {
                    MONOSPACED
                } else {
                    PROPORTIONAL
                }));
            }
            rows.push(Row::Family(family.clone()));
        }
        rows
    }

    fn open_list(&mut self, which: List, window: &mut Window, cx: &mut Context<Self>) {
        let rows = Rc::new(Self::rows(which, cx));
        let current: Option<SharedString> = match which {
            List::Family => Some(self.family.clone().into()),
            List::Fallback => self.fallback.clone().map(Into::into),
        };
        let highlighted = rows.iter().position(|r| r.value() == Some(current.clone()));
        let scroll = UniformListScrollHandle::new();
        if let Some(ix) = highlighted {
            scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.list = Some(OpenList {
            which,
            rows,
            highlighted,
            scroll,
            focus,
        });
        cx.notify();
    }

    fn close_list(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            if let Some(window) = window {
                window.focus(&self.focus);
            }
            cx.notify();
        }
    }

    fn toggle_list(&mut self, which: List, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.as_ref().is_some_and(|l| l.which == which) {
            self.close_list(Some(window), cx);
        } else {
            self.open_list(which, window, cx);
        }
    }

    /// Takes the value of row `ix` of the open list and closes it.
    fn choose(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(list) = &self.list else {
            return;
        };
        let which = list.which;
        let Some(value) = list.rows.get(ix).and_then(Row::value) else {
            return;
        };
        match which {
            List::Family => {
                if let Some(family) = value {
                    self.family = family.to_string();
                }
            }
            List::Fallback => self.fallback = value.map(|v| v.to_string()),
        }
        self.close_list(Some(window), cx);
        self.sync_preview(cx);
    }

    fn move_highlight(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(list) = &mut self.list else {
            return;
        };
        let count = list.rows.len();
        let selectable = |ix: usize| !matches!(list.rows[ix], Row::Header(_));
        let mut ix = list.highlighted;
        for _ in 0..count {
            let next = match (ix, down) {
                (None, true) => 0,
                (None, false) => count.saturating_sub(1),
                (Some(i), true) => (i + 1) % count,
                (Some(i), false) => (i + count - 1) % count,
            };
            ix = Some(next);
            if selectable(next) {
                break;
            }
        }
        if let Some(ix) = ix.filter(|ix| *ix < count) {
            list.highlighted = Some(ix);
            list.scroll.scroll_to_item(ix, ScrollStrategy::Top);
        }
        cx.notify();
    }

    fn list_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.move_highlight(true, cx),
            "up" => self.move_highlight(false, cx),
            "enter" => {
                if let Some(ix) = self.list.as_ref().and_then(|l| l.highlighted) {
                    self.choose(ix, window, cx);
                }
            }
            "escape" => self.close_list(Some(window), cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    // ---- drawing ----

    fn combo(
        &self,
        which: List,
        text: AnyElement,
        width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.list.as_ref().is_some_and(|l| l.which == which);
        let anchors = self.anchors.clone();
        let focus = cx.theme().color(UiRole::Focus);
        ui::form::combo(
            SharedString::from(format!("font-{}", which.name())),
            vec![text],
            cx,
        )
        .debug_selector(move || format!("font:{}", which.name()))
        .h(px(ROW))
        .w(px(width))
        .when(open, |d| d.border_color(focus))
        .child(
            canvas(
                move |bounds, _, _| anchors.borrow_mut()[which.index()] = bounds,
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .on_click(
            cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_list(which, window, cx)
            }),
        )
        .into_any_element()
    }

    /// The family's text in its combo box: a family the text system does
    /// not have in the error colour (`MyWarningItem`).
    fn family_text(&self, cx: &App) -> AnyElement {
        let (list, loaded) = families(cx);
        let missing = loaded && !list.iter().any(|f| f.name.as_ref() == self.family);
        if missing {
            div()
                .text_color(cx.theme().color(UiRole::Danger))
                .child(format!("<Font missing: {}>", self.family))
                .into_any_element()
        } else {
            div().child(self.family.clone()).into_any_element()
        }
    }

    fn number_field(
        &self,
        line_height: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = if line_height {
            &self.line_height
        } else {
            &self.size
        };
        let range = if line_height {
            EditorFontSettings::LINE_HEIGHTS
        } else {
            EditorFontSettings::SIZES
        };
        let error = parse_number(&Self::text(field, cx), &range).is_err();
        let name = if line_height {
            field::LINE_HEIGHT
        } else {
            field::SIZE
        };
        div()
            .id(name)
            .debug_selector(move || format!("font:{name}"))
            .flex_none()
            .w(px(NUMBER_WIDTH))
            .capture_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                let up = match e.keystroke.key.as_str() {
                    "up" => true,
                    "down" => false,
                    _ => return,
                };
                this.step(line_height, up, cx);
                cx.stop_propagation();
            }))
            .child(
                editor::field::frame(field, true, error, window, cx)
                    .h(px(ROW))
                    .pl(px(6.0))
                    .pr(px(4.0)),
            )
            .into_any_element()
    }

    fn render_list(&self, list: &OpenList, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let bounds = self.anchors.borrow()[list.which.index()];
        let shown_rows = list.rows.len().clamp(1, LIST_ROWS);
        let height = LIST_ROW * shown_rows as f32 + 2.0 * LIST_PAD;
        let rows = list.rows.clone();
        let highlighted = list.highlighted;
        let (_, loaded) = families(cx);
        let body = uniform_list(
            "font-list",
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let theme = cx.theme();
                let selection = theme.color(UiRole::Selection);
                let muted = theme.color(UiRole::TextMuted);
                let small = theme.ui_size() - px(1.0);
                range
                    .map(|ix| {
                        let row = div()
                            .id(ElementId::NamedInteger("font-row".into(), ix as u64))
                            .h(px(LIST_ROW))
                            .px(px(LIST_PAD))
                            .flex()
                            .items_center();
                        match &rows[ix] {
                            Row::Header(title) => row
                                .debug_selector(move || format!("font-header:{title}"))
                                .text_size(small)
                                .font_weight(FontWeight::BOLD)
                                .text_color(muted)
                                .child(div().px(px(LIST_PAD)).child(*title)),
                            other => {
                                let (label, family) = match other {
                                    Row::Family(f) => (
                                        f.name.clone(),
                                        (loaded && f.shows_name).then(|| f.name.clone()),
                                    ),
                                    _ => (SharedString::from(NONE), None),
                                };
                                let selector = format!("font-row:{label}");
                                row.debug_selector(move || selector.clone())
                                    .pointing_hand()
                                    .child(
                                        div()
                                            .flex_1()
                                            .h_full()
                                            .px(px(LIST_PAD))
                                            .flex()
                                            .items_center()
                                            .rounded(px(4.0))
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .when(highlighted == Some(ix), |d| d.bg(selection))
                                            .when_some(family, |d, f| d.font_family(f))
                                            .child(label),
                                    )
                                    .on_click(cx.listener(
                                        move |this: &mut Self, _: &ClickEvent, window, cx| {
                                            this.choose(ix, window, cx)
                                        },
                                    ))
                            }
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(list.scroll.clone())
        .h(px(height - 2.0 * LIST_PAD));
        let popup = div()
            .id("font-list-popup")
            .debug_selector(|| "font-list".into())
            .key_context(ui::menu::KEY_CONTEXT)
            .track_focus(&list.focus)
            .occlude()
            .w(bounds.size.width.max(px(FALLBACK_WIDTH)))
            .h(px(height))
            .py(px(LIST_PAD))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .on_key_down(cx.listener(Self::list_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_list(Some(window), cx)))
            .child(body);
        deferred(
            anchored()
                .position(bounds.bottom_left() + gpui::point(px(0.0), px(2.0)))
                .snap_to_window()
                .child(popup),
        )
        .with_priority(1)
        .into_any_element()
    }

    fn render_form(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let divider = theme.color(UiRole::Divider);
        let disabled = theme.color(UiRole::TextDisabled);
        let small = theme.ui_size() - px(1.0);
        let label = |text: &'static str, cx: &App| {
            div()
                .flex_none()
                .w(px(LABEL_COLUMN))
                .child(crate::search::label(text, cx))
        };
        let family_text = self.family_text(cx);
        let family_row = div()
            .flex()
            .items_center()
            .h(px(ROW))
            .child(label(FONT, cx))
            .child(self.combo(List::Family, family_text, FAMILY_WIDTH, cx));
        let size_row = div()
            .flex()
            .items_center()
            .h(px(ROW))
            .child(label(SIZE, cx))
            .child(self.number_field(false, window, cx))
            .child(div().w(px(SIZE_GAP)))
            .child(crate::search::label(LINE_HEIGHT, cx))
            .child(div().w(px(LABEL_GAP)))
            .child(self.number_field(true, window, cx));
        let ligatures = ui::form::check_row(
            "font-ligatures-row",
            "font-ligatures-check",
            self.ligatures,
            div().whitespace_nowrap().child(LIGATURES),
            cx,
        )
        .debug_selector(|| "font:ligatures".into())
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_ligatures(cx)));
        let ligatures_row = div()
            .flex()
            .items_center()
            .h(px(ROW))
            .child(crate::search::spotlight(LIGATURES, ligatures, cx))
            .child(div().w(px(HELP_GAP)))
            .child(
                div()
                    .id("font-ligatures-help")
                    .child(Icon::new(IconName::CONTEXT_HELP))
                    .tooltip(Tooltip::text(LIGATURES_TOOLTIP, None)),
            );
        let reader_comment = div()
            .flex()
            .flex_row()
            .items_center()
            .text_size(small)
            .text_color(muted)
            .debug_selector(|| "font:reader-mode".into())
            .child(READER_COMMENT)
            .child(
                ui::link::link("font-reader-mode", READER_LINK, cx)
                    .debug_selector(|| "font:reader-mode-link".into())
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::commands::SelectPage {
                                page: READER_PAGE.to_owned(),
                            }),
                            cx,
                        )
                    }),
            );
        let restore = if self.restorable(cx) {
            ui::link::link("font-restore", RESTORE, cx)
                .debug_selector(|| "font:restore-defaults".into())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.restore_defaults(cx)))
                .into_any_element()
        } else {
            div()
                .debug_selector(|| "font:restore-defaults".into())
                .text_color(disabled)
                .child(RESTORE)
                .into_any_element()
        };
        let expanded = self.typography;
        let typography_header = div()
            .id("font-typography")
            .debug_selector(|| "font:typography".into())
            .flex()
            .items_center()
            .h(px(ROW))
            .pointing_hand()
            .child(ui::components::tree_arrow(expanded, cx))
            .child(div().w(px(4.0)))
            .child(crate::search::label(TYPOGRAPHY, cx))
            .child(div().flex_1().ml(px(6.0)).h(px(1.0)).bg(divider))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.typography = !this.typography;
                cx.notify();
            }));
        let fallback_text = div()
            .child(self.fallback.clone().unwrap_or_else(|| NONE.to_owned()))
            .into_any_element();
        let typography = expanded.then(|| {
            let comment = FALLBACK_COMMENT.lines().map(|line| div().child(line));
            div()
                .pl(px(GROUP_INDENT))
                .pt(px(SMALL_GAP))
                .flex()
                .flex_row()
                .child(
                    div()
                        .flex_none()
                        .h(px(ROW))
                        .flex()
                        .items_center()
                        .child(crate::search::label(FALLBACK, cx)),
                )
                .child(div().w(px(LABEL_GAP)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(self.combo(List::Fallback, fallback_text, FALLBACK_WIDTH, cx))
                        .child(
                            div()
                                .pt(px(COMMENT_TOP))
                                .text_size(small)
                                .text_color(muted)
                                .children(comment),
                        ),
                )
        });
        div()
            .id("font-options")
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p(px(PAD))
            .flex()
            .flex_col()
            .child(family_row)
            .child(div().h(px(ROW_GAP)))
            .child(size_row)
            .child(div().h(px(LIGATURES_TOP)))
            .child(ligatures_row)
            .child(div().h(px(READER_TOP)))
            .child(reader_comment)
            .child(div().h(px(RESTORE_TOP)))
            .child(div().flex().child(restore))
            .child(div().h(px(GROUP_TOP)))
            .child(typography_header)
            .children(typography)
            .into_any_element()
    }

    fn render_preview(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        div()
            .id("font-preview")
            .debug_selector(|| "font:preview".into())
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(theme.color(UiRole::Divider))
            .bg(theme.color(UiRole::EditorBackground))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .pl(px(PREVIEW_LEFT))
                    .child(self.preview.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .py(px(HINT_PAD.0))
                    .pl(px(HINT_PAD.1))
                    .text_size(theme.ui_size() - px(1.0))
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(PREVIEW_HINT),
            )
            .into_any_element()
    }
}

impl SettingsPage for FontPage {
    fn is_modified(&self, cx: &App) -> bool {
        match self.shown(cx) {
            Ok(shown) => shown.settings(cx) != Shown::stored(cx).settings(cx),
            Err(_) => true,
        }
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let shown = self.shown(cx)?;
        let settings = shown.settings(cx);
        store::set(cx, &settings);
        // The fields show the numbers kept in range.
        self.set_numbers(shown.size, shown.line_height, cx);
        self.sync_preview(cx);
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stored = Shown::stored(cx);
        self.family = stored.family;
        self.ligatures = stored.ligatures;
        self.fallback = stored.fallback;
        self.set_numbers(stored.size, stored.line_height, cx);
        self.close_list(Some(window), cx);
        self.sync_preview(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        let (list, loaded) = families(cx);
        let preview = self.preview.read(cx);
        let font = self.preview_shown(cx);
        json!({
            "family": self.family,
            "size": Self::text(&self.size, cx),
            "line-height": Self::text(&self.line_height, cx),
            "ligatures": self.ligatures,
            "fallback": self.fallback,
            "restore-defaults": self.restorable(cx),
            "typography": self.typography,
            "families": list.len(),
            "families-loaded": loaded,
            "list": self.list.as_ref().map(|l| l.which.name()),
            "rows": self.list.as_ref().map(|l| {
                l.rows
                    .iter()
                    .map(|r| match r {
                        Row::Header(t) => json!({"header": t}),
                        Row::None => json!({"label": NONE}),
                        Row::Family(f) => json!({"label": f.name, "monospaced": f.monospaced}),
                    })
                    .collect::<Vec<_>>()
            }),
            "highlighted": self.list.as_ref().and_then(|l| l.highlighted),
            "preview": {
                "family": font.family,
                "size": font.size,
                "line-height": font.line_height,
                "ligatures": font.ligatures,
                "fallback": font.fallback,
                "text-lines": preview.buffer().len_lines(),
            },
        })
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let text = || match value {
            Json::String(s) => Ok(s.clone()),
            Json::Number(n) => Ok(n.to_string()),
            _ => Err(format!("`{name}` takes a string or a number")),
        };
        match name {
            field::FAMILY => {
                self.family = value
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("`family` takes a family name")?
                    .to_owned();
            }
            field::FALLBACK => {
                self.fallback = match value {
                    Json::Null => None,
                    Json::String(s) if s == NONE || s.is_empty() => None,
                    Json::String(s) => Some(s.clone()),
                    _ => return Err("`fallback` takes a family name or null".into()),
                };
            }
            field::SIZE => {
                let t = text()?;
                self.size.update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            field::LINE_HEIGHT => {
                let t = text()?;
                self.line_height
                    .update(cx, |e, cx| e.set_text_selected(&t, cx));
            }
            field::LIGATURES => {
                self.ligatures = value.as_bool().ok_or("`ligatures` takes true or false")?;
            }
            field::RESTORE => self.restore_defaults(cx),
            field::TYPOGRAPHY => {
                self.typography = value.as_bool().ok_or("`typography` takes true or false")?;
            }
            // Opens (`"family"`, `"fallback"`) or closes (`null`) a list.
            field::LIST => match value.as_str() {
                Some("family") => self.open_list(List::Family, window, cx),
                Some("fallback") => {
                    self.typography = true;
                    self.open_list(List::Fallback, window, cx)
                }
                None => self.close_list(Some(window), cx),
                Some(other) => return Err(format!("no list `{other}`")),
            },
            // Chooses a row of the open list by its label.
            field::CHOOSE => {
                let label = value.as_str().ok_or("`choose` takes a row's label")?;
                let ix = self
                    .list
                    .as_ref()
                    .and_then(|l| {
                        l.rows.iter().position(|r| match r {
                            Row::None => label == NONE,
                            Row::Family(f) => f.name.as_ref() == label,
                            Row::Header(_) => false,
                        })
                    })
                    .ok_or_else(|| format!("no row `{label}` in an open list"))?;
                self.choose(ix, window, cx);
            }
            _ => return Err(format!("no field `{name}` on this page")),
        }
        self.sync_preview(cx);
        cx.notify();
        Ok(())
    }
}

impl Render for FontPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let divider = cx.theme().color(UiRole::Divider);
        let form = self.render_form(window, cx);
        let preview = self.render_preview(cx);
        let list = self.list.as_ref().map(|l| self.render_list(l, cx));
        div()
            .id("font-page")
            .debug_selector(|| "font-page".into())
            .track_focus(&self.focus)
            // The page has no margin (`Configurable.NoMargin`): it fills
            // the page area under the banner.
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .flex_row()
            .border_t_1()
            .border_color(divider)
            .child(form)
            .child(preview)
            .children(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_text_has_its_bold_parts() {
        let (text, bold) = preview_text();
        assert!(text.starts_with("Pyoxide is an Integrated \nDevelopment"));
        assert!(!text.contains("<bold>") && !text.contains("</bold>"));
        let part = |r: &Range<usize>| text.chars().skip(r.start).take(r.len()).collect::<String>();
        assert_eq!(part(&bold[0]), "Integrated \nDevelopment Environment (IDE)");
        assert!(part(&bold[2]).starts_with("Bold:\n"));
        assert!(text.contains("<!-- -- != := ==="));

        let mut buffer = Buffer::new(&text);
        let mut h = PreviewHighlighter {
            bold,
            lines: Vec::new(),
        };
        h.reset(&buffer);
        assert_eq!(h.lines[0], [(0..14, false), (14..25, true)]);
        assert_eq!(h.lines[1], [(0..29, true), (29..38, false)]);
        // Typing before a bold part moves it.
        let edit = buffer.replace_untracked(0..0, "Hi ");
        h.edited(&buffer, &[edit]);
        assert_eq!(h.lines[0], [(0..17, false), (17..28, true)]);
    }

    #[test]
    fn numbers_as_the_reference_shows_them() {
        assert_eq!(number_text(14.0), "14.0");
        assert_eq!(number_text(1.2), "1.2");
        assert_eq!(number_text(13.5), "13.5");
        assert_eq!(parse_number(" 100 ", &EditorFontSettings::SIZES), Ok(40.0));
        assert_eq!(
            parse_number("0.1", &EditorFontSettings::LINE_HEIGHTS),
            Ok(0.6)
        );
        assert!(parse_number("big", &EditorFontSettings::SIZES).is_err());
    }

    #[test]
    fn monospaced_families_come_first() {
        let family = |name: &str, monospaced| Family {
            name: name.to_owned().into(),
            monospaced,
            shows_name: true,
        };
        let mut list = vec![
            family("Arial", false),
            family("Menlo", true),
            family("Courier", true),
        ];
        sort_families(&mut list);
        let names: Vec<_> = list.iter().map(|f| f.name.to_string()).collect();
        assert_eq!(names, ["Courier", "Menlo", "Arial"]);
    }
}
