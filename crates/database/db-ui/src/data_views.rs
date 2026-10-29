//! Settings > Database > Data Views: the patterns the grid shows dates and
//! times with and how it writes numbers (`[database.data-views]`, see
//! [`DataFormats`]). The open grids follow the store when the page is
//! applied.

use chrono::{NaiveDate, NaiveTime};
use editor::{Editor, EditorEvent};
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FocusHandle, IntoElement, Render,
    Subscription, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::{PageContext, PageSpec, Scope, SettingsPage, store};
use themes::UiRole;
use ui::components::DISABLED_ALPHA;
use ui::theme::ThemeExt;

use crate::grid::formats::DataFormats;

pub const DATABASE: &str = "database";
pub const DATA_VIEWS: &str = "database.data-views";

const DATABASE_DESCRIPTION: &str =
    "Configure how the database tools connect to data sources and show their data.";
const DATE_AND_TIME: &str = "Date and Time";
const NUMBERS: &str = "Numbers";
const DATE: &str = "Date:";
const TIME: &str = "Time:";
const TIMESTAMP: &str = "Timestamp:";
const TIMESTAMP_TZ: &str = "Timestamp with time zone:";
const GROUP_NUMBERS: &str = "Group digits of numbers";
const GROUP_SEPARATOR: &str = "Group separator:";
const DECIMAL_SEPARATOR: &str = "Decimal separator:";
const PATTERN_HINT: &str =
    "Letters of the SimpleDateFormat patterns: yyyy MM dd HH mm ss SSS, text in single quotes";
const EXAMPLE: &str = "Example:";
const SEARCH_TERMS: &[&str] = &[
    DATE_AND_TIME,
    NUMBERS,
    DATE,
    TIME,
    TIMESTAMP,
    TIMESTAMP_TZ,
    GROUP_NUMBERS,
    GROUP_SEPARATOR,
    DECIMAL_SEPARATOR,
];

const GROUP_INDENT: f32 = 20.0;
const ROW_GAP: f32 = 6.0;
const ROW_HEIGHT: f32 = 22.0;
const LABEL_WIDTH: f32 = 190.0;
const PATTERN_WIDTH: f32 = 280.0;
const SEPARATOR_WIDTH: f32 = 60.0;
const FIELD_HEIGHT: f32 = 24.0;

/// The group and the page.
pub fn pages() -> Vec<PageSpec> {
    vec![
        PageSpec {
            weight: 60,
            description: Some(DATABASE_DESCRIPTION),
            ..PageSpec::group(DATABASE, None, "Database")
        },
        PageSpec {
            id: DATA_VIEWS,
            parent: Some(DATABASE),
            title: "Data Views",
            weight: 0,
            scope: Scope::Application,
            search_terms: SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(DataViewsPage::new)),
        },
    ]
}

/// The keys of `settings.edit`.
mod field {
    pub const DATE: &str = "date";
    pub const TIME: &str = "time";
    pub const TIMESTAMP: &str = "timestamp";
    pub const TIMESTAMP_TZ: &str = "timestamp-tz";
    pub const GROUP_NUMBERS: &str = "group-numbers";
    pub const GROUP_SEPARATOR: &str = "group-separator";
    pub const DECIMAL_SEPARATOR: &str = "decimal-separator";
}

pub struct DataViewsPage {
    date: Entity<Editor>,
    time: Entity<Editor>,
    timestamp: Entity<Editor>,
    timestamp_tz: Entity<Editor>,
    group_numbers: bool,
    group_separator: Entity<Editor>,
    decimal_separator: Entity<Editor>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl DataViewsPage {
    pub fn new(_: &PageContext, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stored: DataFormats = store::get(cx);
        let new = |text: &str, window: &mut Window, cx: &mut Context<Self>| {
            editor::field::new(text, None, false, window, cx)
        };
        let date = new(&stored.date, window, cx);
        let time = new(&stored.time, window, cx);
        let timestamp = new(&stored.timestamp, window, cx);
        let timestamp_tz = new(&stored.timestamp_tz, window, cx);
        let group_separator = new(&stored.group_separator, window, cx);
        let decimal_separator = new(&stored.decimal_separator, window, cx);
        let edited = |_: &mut Self, _: Entity<Editor>, e: &EditorEvent, cx: &mut Context<Self>| {
            if matches!(e, EditorEvent::Edited(_)) {
                cx.notify();
            }
        };
        let subscriptions = [
            &date,
            &time,
            &timestamp,
            &timestamp_tz,
            &group_separator,
            &decimal_separator,
        ]
        .into_iter()
        .map(|e| cx.subscribe(e, edited))
        .collect();
        Self {
            date,
            time,
            timestamp,
            timestamp_tz,
            group_numbers: stored.group_numbers,
            group_separator,
            decimal_separator,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn text(field: &Entity<Editor>, cx: &App) -> String {
        field.read(cx).text()
    }

    /// The draft; the first problem is the error.
    fn draft(&self, cx: &App) -> Result<DataFormats, String> {
        let pattern = |field: &Entity<Editor>, name: &str| {
            let text = Self::text(field, cx);
            match DataFormats::check_pattern(&text) {
                Some(why) => Err(format!("{name}: {why}")),
                None => Ok(text),
            }
        };
        let decimal = Self::text(&self.decimal_separator, cx);
        let group = Self::text(&self.group_separator, cx);
        if decimal.is_empty() {
            return Err("The decimal separator is empty".to_owned());
        }
        if self.group_numbers && decimal == group {
            return Err("The separators of groups and of the fraction are the same".to_owned());
        }
        Ok(DataFormats {
            date: pattern(&self.date, "Date")?,
            time: pattern(&self.time, "Time")?,
            timestamp: pattern(&self.timestamp, "Timestamp")?,
            timestamp_tz: pattern(&self.timestamp_tz, "Timestamp with time zone")?,
            group_numbers: self.group_numbers,
            group_separator: group,
            decimal_separator: decimal,
        })
    }

    fn set_text(field: &Entity<Editor>, text: &str, cx: &mut Context<Self>) {
        field.update(cx, |e, cx| e.set_text_selected(text, cx));
    }

    fn field_of(&self, name: &str) -> Option<&Entity<Editor>> {
        Some(match name {
            field::DATE => &self.date,
            field::TIME => &self.time,
            field::TIMESTAMP => &self.timestamp,
            field::TIMESTAMP_TZ => &self.timestamp_tz,
            field::GROUP_SEPARATOR => &self.group_separator,
            field::DECIMAL_SEPARATOR => &self.decimal_separator,
            _ => return None,
        })
    }

    fn row(
        &self,
        label: &'static str,
        field: &Entity<Editor>,
        width: f32,
        enabled: bool,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let error =
            DataFormats::check_pattern(&Self::text(field, cx)).is_some() && width == PATTERN_WIDTH;
        div()
            .flex()
            .flex_row()
            .items_center()
            .min_h(px(ROW_HEIGHT))
            .when(!enabled, |d| d.opacity(DISABLED_ALPHA))
            .child(
                div()
                    .flex_none()
                    .w(px(LABEL_WIDTH))
                    .whitespace_nowrap()
                    .child(settings_ui::search::label(label, cx)),
            )
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(width))
                    .child(
                        editor::field::frame(field, false, error, window, cx).h(px(FIELD_HEIGHT)),
                    )
                    .when(!enabled, |d| {
                        d.child(
                            div()
                                .id(ElementId::Name(format!("data-views-off:{label}").into()))
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .occlude(),
                        )
                    }),
            )
            .into_any_element()
    }

    /// A sample of what the draft makes of values.
    fn example(&self, cx: &App) -> Option<(String, String)> {
        let f = self.draft(cx).ok()?;
        let day = NaiveDate::from_ymd_opt(2024, 3, 9)?;
        let at = NaiveTime::from_hms_micro_opt(14, 5, 7, 123_456)?;
        Some((
            format!(
                "{}   {}   {}",
                f.format_date(day),
                f.format_time(at),
                f.format_timestamp(day.and_time(at))
            ),
            f.number("-1234567.891"),
        ))
    }
}

impl SettingsPage for DataViewsPage {
    fn is_modified(&self, cx: &App) -> bool {
        match self.draft(cx) {
            Ok(draft) => draft != store::get::<DataFormats>(cx),
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
        let stored: DataFormats = store::get(cx);
        Self::set_text(&self.date, &stored.date, cx);
        Self::set_text(&self.time, &stored.time, cx);
        Self::set_text(&self.timestamp, &stored.timestamp, cx);
        Self::set_text(&self.timestamp_tz, &stored.timestamp_tz, cx);
        Self::set_text(&self.group_separator, &stored.group_separator, cx);
        Self::set_text(&self.decimal_separator, &stored.decimal_separator, cx);
        self.group_numbers = stored.group_numbers;
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, cx: &App) -> Json {
        json!({
            "date": Self::text(&self.date, cx),
            "time": Self::text(&self.time, cx),
            "timestamp": Self::text(&self.timestamp, cx),
            "timestamp-tz": Self::text(&self.timestamp_tz, cx),
            "group-numbers": self.group_numbers,
            "group-separator": Self::text(&self.group_separator, cx),
            "decimal-separator": Self::text(&self.decimal_separator, cx),
            "error": self.draft(cx).err(),
            "example": self.example(cx).map(|(dates, number)| json!({"dates": dates, "number": number})),
        })
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if name == field::GROUP_NUMBERS {
            self.group_numbers = value
                .as_bool()
                .ok_or_else(|| format!("`{name}` takes true or false"))?;
        } else {
            let field = self
                .field_of(name)
                .ok_or_else(|| format!("no field `{name}` on this page"))?
                .clone();
            let text = value
                .as_str()
                .ok_or_else(|| format!("`{name}` takes a string"))?;
            Self::set_text(&field, text, cx);
        }
        cx.notify();
        Ok(())
    }
}

impl Render for DataViewsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.color(UiRole::TextMuted);
        let small = theme.ui_size() - px(1.0);
        let on = self.group_numbers;
        let group_check = settings_ui::search::spotlight(
            GROUP_NUMBERS,
            ui::form::check_row(
                "data-views-row:group",
                "data-views-check:group",
                on,
                div().whitespace_nowrap().child(GROUP_NUMBERS),
                cx,
            )
            .min_h(px(ROW_HEIGHT))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.group_numbers = !this.group_numbers;
                cx.notify();
            })),
            cx,
        );
        let example = self.example(cx);
        let error = self.draft(cx).err();
        let date_rows: Vec<AnyElement> = vec![
            self.row(DATE, &self.date, PATTERN_WIDTH, true, window, cx),
            self.row(TIME, &self.time, PATTERN_WIDTH, true, window, cx),
            self.row(TIMESTAMP, &self.timestamp, PATTERN_WIDTH, true, window, cx),
            self.row(
                TIMESTAMP_TZ,
                &self.timestamp_tz,
                PATTERN_WIDTH,
                true,
                window,
                cx,
            ),
            div()
                .text_size(small)
                .text_color(muted)
                .child(PATTERN_HINT)
                .into_any_element(),
        ];
        let number_rows: Vec<AnyElement> = vec![
            group_check,
            self.row(
                GROUP_SEPARATOR,
                &self.group_separator,
                SEPARATOR_WIDTH,
                on,
                window,
                cx,
            ),
            self.row(
                DECIMAL_SEPARATOR,
                &self.decimal_separator,
                SEPARATOR_WIDTH,
                true,
                window,
                cx,
            ),
        ];
        div()
            .id("data-views-page")
            .debug_selector(|| "data-views-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(ui::form::group_header(DATE_AND_TIME, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .children(date_rows),
            )
            .child(ui::form::group_header(NUMBERS, cx))
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .flex()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .children(number_rows),
            )
            .child(
                div()
                    .pl(px(GROUP_INDENT))
                    .text_size(small)
                    .text_color(muted)
                    .child(match (&error, &example) {
                        (Some(why), _) => why.clone(),
                        (None, Some((dates, number))) => format!("{EXAMPLE} {dates}   {number}"),
                        _ => String::new(),
                    }),
            )
    }
}
