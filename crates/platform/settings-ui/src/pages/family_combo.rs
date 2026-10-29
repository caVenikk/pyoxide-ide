//! The font family combo box of the font settings (the reference's
//! `FontComboBox` / `FontFamilyCombo`), shared by Editor > Font and the
//! font group of Tools > Terminal: a combo whose list holds the families of
//! the text system, listed and probed (monospaced, able to show its own
//! name) on the background executor, monospaced ones first.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, ElementId, EventEmitter, FocusHandle, FontWeight,
    Global, IntoElement, KeyDownEvent, Pixels, Render, ScrollStrategy, SharedString, Task,
    TextSystem, UniformListScrollHandle, Window, anchored, canvas, div, prelude::*, px,
    uniform_list,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::deferred;
use ui::pointer::PointerExt as _;
use ui::theme::ThemeExt;

/// The combo box's height (a row of the page).
pub const ROW: f32 = 24.0;
/// The list of a combo box: rows of `List.rowHeight`, at most this many
/// shown, the body's insets.
const LIST_ROW: f32 = 24.0;
const LIST_ROWS: usize = 15;
const LIST_PAD: f32 = 8.0;

const MONOSPACED: &str = "Monospaced";
const PROPORTIONAL: &str = "Proportional";

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
pub(super) fn sort_families(families: &mut [Family]) {
    families.sort_by(|a, b| {
        b.monospaced
            .cmp(&a.monospaced)
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Lists the families on the background executor, once per application;
/// the windows redraw when they are there.
pub fn refresh_families(cx: &mut App) {
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

/// The families known so far, and whether the listing is done.
pub fn families(cx: &App) -> (Arc<Vec<Family>>, bool) {
    cx.try_global::<Families>()
        .map(|f| (f.list.clone(), f.loaded))
        .unwrap_or_default()
}

// ---- the combo box ---------------------------------------------------------------------

/// How a combo box looks and what its list holds.
#[derive(Debug, Clone, Copy)]
pub struct ComboSpec {
    /// The prefix of element ids and debug selectors (`font`,
    /// `terminal-font`).
    pub prefix: &'static str,
    /// The combo's name (`family`, `fallback`).
    pub name: &'static str,
    pub width: f32,
    /// The list is at least this wide.
    pub list_width: f32,
    /// Only monospaced families are listed (`isMonospacedOnly`).
    pub monospaced_only: bool,
    /// The Monospaced and Proportional headers divide the list.
    pub groups: bool,
    /// Families that cannot draw their own name are left out.
    pub names_only: bool,
    /// The label of the first row, which stands for no family chosen
    /// (`<None>`, `Monospaced`); `None`: every row is a family.
    pub none: Option<&'static str>,
    /// A family the text system lacks shows as `<Font missing: …>` in the
    /// error colour (`MyWarningItem`).
    pub warn_missing: bool,
}

/// What a combo box tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboEvent {
    /// A row of the list was chosen.
    Chosen,
}

impl EventEmitter<ComboEvent> for FamilyCombo {}

/// A row of the list.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(&'static str),
    None(&'static str),
    Family(Family),
}

impl Row {
    /// The value the row stands for; `None` for a header.
    fn value(&self) -> Option<Option<SharedString>> {
        match self {
            Row::Header(_) => None,
            Row::None(_) => Some(None),
            Row::Family(f) => Some(Some(f.name.clone())),
        }
    }

    fn label(&self) -> &str {
        match self {
            Row::Header(t) | Row::None(t) => t,
            Row::Family(f) => f.name.as_ref(),
        }
    }
}

/// An open list: its rows, the highlighted one and its scroll.
struct OpenList {
    rows: Rc<Vec<Row>>,
    highlighted: Option<usize>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

/// A combo box of font families: the chosen family (`None`: the none row),
/// and the list that opens below it.
pub struct FamilyCombo {
    spec: ComboSpec,
    value: Option<String>,
    list: Option<OpenList>,
    /// Where the combo is drawn (the list opens below it).
    anchor: Rc<Cell<Bounds<Pixels>>>,
    /// Takes the focus when the list closes.
    owner_focus: FocusHandle,
}

impl FamilyCombo {
    pub fn new(
        spec: ComboSpec,
        value: Option<String>,
        owner_focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        refresh_families(cx);
        Self {
            spec,
            value,
            list: None,
            anchor: Rc::default(),
            owner_focus,
        }
    }

    pub fn spec(&self) -> &ComboSpec {
        &self.spec
    }

    /// The chosen family; `None` for the none row.
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// Sets the family (the owner's reset or edit); no event.
    pub fn set_value(&mut self, value: Option<String>, cx: &mut Context<Self>) {
        if self.value != value {
            self.value = value;
            cx.notify();
        }
    }

    pub fn is_open(&self) -> bool {
        self.list.is_some()
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let (list, _) = families(cx);
        let mut rows = Vec::new();
        if let Some(none) = self.spec.none {
            rows.push(Row::None(none));
        }
        let mut monospaced = None;
        for family in list.iter() {
            if (self.spec.names_only && !family.shows_name)
                || (self.spec.monospaced_only && !family.monospaced)
            {
                continue;
            }
            if self.spec.groups && monospaced != Some(family.monospaced) {
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

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = Rc::new(self.rows(cx));
        let current: Option<SharedString> = self.value.clone().map(Into::into);
        let highlighted = rows.iter().position(|r| r.value() == Some(current.clone()));
        let scroll = UniformListScrollHandle::new();
        if let Some(ix) = highlighted {
            scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.list = Some(OpenList {
            rows,
            highlighted,
            scroll,
            focus,
        });
        cx.notify();
    }

    pub fn close(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.list.take().is_some() {
            if let Some(window) = window {
                window.focus(&self.owner_focus);
            }
            cx.notify();
        }
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.is_some() {
            self.close(Some(window), cx);
        } else {
            self.open(window, cx);
        }
    }

    /// Takes the value of row `ix` of the open list and closes it.
    fn choose(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self
            .list
            .as_ref()
            .and_then(|l| l.rows.get(ix))
            .and_then(Row::value)
        else {
            return;
        };
        self.value = value.map(|v| v.to_string());
        self.close(Some(window), cx);
        cx.emit(ComboEvent::Chosen);
    }

    /// Chooses the row of the open list labelled `label`
    /// (`settings.edit` `choose`).
    pub fn choose_label(
        &mut self,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let ix = self
            .list
            .as_ref()
            .and_then(|l| {
                l.rows
                    .iter()
                    .position(|r| !matches!(r, Row::Header(_)) && r.label() == label)
            })
            .ok_or_else(|| format!("no row `{label}` in an open list"))?;
        self.choose(ix, window, cx);
        Ok(())
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
            "escape" => self.close(Some(window), cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The combo's state for `settings.state`: the family and, while the
    /// list is open, its rows and the highlighted one.
    pub fn state(&self) -> Json {
        json!({
            "value": self.value,
            "open": self.list.is_some(),
            "rows": self.list.as_ref().map(|l| {
                l.rows
                    .iter()
                    .map(|r| match r {
                        Row::Header(t) => json!({"header": t}),
                        Row::None(t) => json!({"label": t}),
                        Row::Family(f) => json!({"label": f.name, "monospaced": f.monospaced}),
                    })
                    .collect::<Vec<_>>()
            }),
            "highlighted": self.list.as_ref().and_then(|l| l.highlighted),
        })
    }

    // ---- drawing ----

    /// The family's text in the combo box.
    fn text(&self, cx: &App) -> AnyElement {
        let Some(value) = &self.value else {
            return div()
                .child(self.spec.none.unwrap_or_default())
                .into_any_element();
        };
        let (list, loaded) = families(cx);
        let missing =
            self.spec.warn_missing && loaded && !list.iter().any(|f| f.name.as_ref() == value);
        if missing {
            div()
                .text_color(cx.theme().color(UiRole::Danger))
                .child(format!("<Font missing: {value}>"))
                .into_any_element()
        } else {
            div().child(value.clone()).into_any_element()
        }
    }

    fn render_list(&self, list: &OpenList, cx: &mut Context<Self>) -> AnyElement {
        let spec = self.spec;
        let theme = cx.theme();
        let bounds = self.anchor.get();
        let shown_rows = list.rows.len().clamp(1, LIST_ROWS);
        let height = LIST_ROW * shown_rows as f32 + 2.0 * LIST_PAD;
        let rows = list.rows.clone();
        let highlighted = list.highlighted;
        let (_, loaded) = families(cx);
        let row_id = SharedString::from(format!("{}-{}-row", spec.prefix, spec.name));
        let body = uniform_list(
            SharedString::from(format!("{}-{}-list", spec.prefix, spec.name)),
            rows.len(),
            cx.processor(move |_, range: Range<usize>, _, cx| {
                let theme = cx.theme();
                let selection = theme.color(UiRole::Selection);
                let muted = theme.color(UiRole::TextMuted);
                let small = theme.ui_size() - px(1.0);
                range
                    .map(|ix| {
                        let row = div()
                            .id(ElementId::NamedInteger(row_id.clone(), ix as u64))
                            .h(px(LIST_ROW))
                            .px(px(LIST_PAD))
                            .flex()
                            .items_center();
                        match &rows[ix] {
                            Row::Header(title) => {
                                let prefix = spec.prefix;
                                row.debug_selector(move || format!("{prefix}-header:{title}"))
                                    .text_size(small)
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(muted)
                                    .child(div().px(px(LIST_PAD)).child(*title))
                            }
                            other => {
                                let family = match other {
                                    Row::Family(f) => {
                                        (loaded && f.shows_name).then(|| f.name.clone())
                                    }
                                    _ => None,
                                };
                                let label = SharedString::from(other.label().to_owned());
                                let selector = format!("{}-row:{label}", spec.prefix);
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
        let prefix = spec.prefix;
        let popup = div()
            .id(SharedString::from(format!(
                "{}-{}-popup",
                spec.prefix, spec.name
            )))
            .debug_selector(move || format!("{prefix}-list"))
            .key_context(ui::menu::KEY_CONTEXT)
            .track_focus(&list.focus)
            .occlude()
            .w(bounds.size.width.max(px(spec.list_width)))
            .h(px(height))
            .py(px(LIST_PAD))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .on_key_down(cx.listener(Self::list_key))
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close(Some(window), cx)))
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
}

impl Render for FamilyCombo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = self.spec;
        let open = self.list.is_some();
        let focus = cx.theme().color(UiRole::Focus);
        let anchor = self.anchor.clone();
        let combo = ui::form::combo(
            SharedString::from(format!("{}-{}", spec.prefix, spec.name)),
            vec![self.text(cx)],
            cx,
        )
        .debug_selector(move || format!("{}:{}", spec.prefix, spec.name))
        .h(px(ROW))
        .w(px(spec.width))
        .when(open, |d| d.border_color(focus))
        .child(
            canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle(window, cx)));
        let list = self.list.as_ref().map(|l| self.render_list(l, cx));
        div()
            .relative()
            .flex_none()
            .w(px(spec.width))
            .h(px(ROW))
            .child(combo)
            .children(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
