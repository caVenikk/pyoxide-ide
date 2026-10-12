//! Drawing Search Everywhere: the tabs and the tab's option over the
//! search field, the results, and a hint at the bottom. With an empty field
//! and nothing to list only the tabs and the field show.

use std::ops::Range;

use file_index::{qualifier_matcher, sanitized_pattern};
use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, HighlightStyle, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, Render, SharedString, StyledText, Window, div, px, uniform_list,
};
use name_match::{Matcher, MatchingMode};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, checkbox, spinner};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::*;

/// Room around the content.
const PAD: f32 = 12.0;

impl SearchEverywhere {
    /// The shortcut of a command where the field has the focus.
    fn shortcut(
        &self,
        action: &dyn gpui::Action,
        window: &Window,
        cx: &App,
    ) -> Option<SharedString> {
        let focus = self.field.focus_handle(cx);
        ui::keys::shortcut_in(action, &focus, window).map(SharedString::from)
    }

    /// The tab's shortcut for its tooltip ("Double ⇧" for All).
    fn tab_shortcut(&self, tab: Tab, window: &Window, cx: &App) -> Option<SharedString> {
        if let Some(key) = crate::keymap::double_tap_for(tab.command(), cx) {
            return Some(crate::keymap::double_tap_text(key, cfg!(target_os = "macos")).into());
        }
        let action = cx.build_action(tab.command(), None).ok()?;
        self.shortcut(action.as_ref(), window, cx)
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let hover = theme.color(UiRole::Hover);
        let tabs: Vec<AnyElement> = Tab::ALL
            .into_iter()
            .map(|tab| {
                let selected = self.tab == tab;
                let id = SharedString::from(format!("search-everywhere-tab-{}", tab.name()));
                div()
                    .id(id.clone())
                    .debug_selector(move || id.to_string())
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(26.0))
                    .px(px(10.0))
                    .rounded(theme.metric(Metric::CornerRadius))
                    .when(selected, |d| {
                        d.bg(theme.color(UiRole::SearchOptionSelected))
                    })
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .child(tab.title())
                    .tooltip(Tooltip::text(
                        tab.title(),
                        self.tab_shortcut(tab, window, cx),
                    ))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.switch_tab(tab, cx)),
                    )
                    .into_any_element()
            })
            .collect();
        let label = match self.tab {
            Tab::Actions => "Include disabled actions",
            _ => "Include non-project items",
        };
        let everywhere = div()
            .id("search-everywhere-everywhere")
            .debug_selector(|| "search-everywhere-everywhere".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .child(checkbox(
                "search-everywhere-everywhere-box",
                self.everywhere,
                cx,
            ))
            .child(label)
            .tooltip(Tooltip::text(
                label,
                self.shortcut(&ToggleEverywhere, window, cx),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_everywhere(cx)));
        div()
            .id("search-everywhere-header")
            .debug_selector(|| "search-everywhere-header".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .h(px(HEADER_HEIGHT))
            .px(px(PAD))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.frame.start_move(e.position);
                    cx.notify();
                }),
            )
            .children(tabs)
            .child(div().flex_1())
            .child(everywhere)
    }

    fn render_field(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("search-everywhere-field")
            .debug_selector(|| "search-everywhere-field".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .h(px(FIELD_HEIGHT))
            .px(px(PAD))
            .when(!self.collapsed(cx), |d| {
                d.border_b_1().border_color(theme.color(UiRole::Divider))
            })
            .child(
                div()
                    .id("search-everywhere-history")
                    .flex_none()
                    .child(Icon::new(IconName::SEARCH_HISTORY).size(px(16.0)))
                    .tooltip(Tooltip::text(
                        "Recent Searches",
                        self.shortcut(&HistoryPrevious, window, cx),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .h(editor::field_line_height(window, cx))
                    .overflow_hidden()
                    .child(self.field.clone()),
            )
            .when(
                self.is_searching() && !self.field.read(cx).text().is_empty(),
                |d| d.child(spinner("search-everywhere-spinner", cx)),
            )
    }

    /// Only the tabs and the field: nothing typed and nothing to list.
    fn collapsed(&self, cx: &App) -> bool {
        self.rows.is_empty() && self.field.read(cx).text().is_empty()
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        let selected = self.selected == Some(index);
        let muted = theme.color(UiRole::TextMuted);
        let base = div()
            .id(("search-everywhere-row", index))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(theme.metric(Metric::RowHeight))
            .px(px(PAD))
            .gap(px(6.0))
            .whitespace_nowrap()
            .overflow_hidden()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select_row(index, cx);
                this.choose(index, window, cx);
            }));
        let base = match row {
            Row::File(file) => {
                let text = self.field.read(cx).text();
                let name = file
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let location = self.location(&file.path);
                // The matched parts show on the selected row only.
                let (name_ranges, location_ranges) = if selected {
                    file_highlights(&text, &name, &location)
                } else {
                    (Vec::new(), Vec::new())
                };
                let icon = if file.is_dir {
                    if file.non_project {
                        IconName::EXCLUDED_FOLDER
                    } else {
                        IconName::FOLDER
                    }
                } else {
                    self.file_types.icon(&file.path)
                };
                let match_bg = theme.color(UiRole::SearchMatch);
                base.when(file.non_project && !selected, |d| {
                    d.bg(theme.color(UiRole::NonProjectFile))
                })
                .child(Icon::new(icon).size(px(16.0)))
                .child(div().flex_none().child(
                    StyledText::new(name.clone()).with_highlights(highlight(
                        &name,
                        &name_ranges,
                        match_bg,
                    )),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .text_color(muted)
                        .child(StyledText::new(location.clone()).with_highlights(highlight(
                            &location,
                            &location_ranges,
                            match_bg,
                        ))),
                )
            }
            Row::Action(i) => {
                let a = &self.context.actions[*i];
                base.when(!a.available, |d| d.text_color(muted))
                    .child(div().flex_none().child(a.title.clone()))
                    .children(a.shortcut.clone().map(|s| {
                        div()
                            .flex_none()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(muted)
                            .child(s)
                    }))
                    .child(div().flex_1())
            }
            Row::More => base.text_size(px(11.0)).text_color(muted).child("… more"),
        };
        base.when(selected, |d| d.bg(theme.color(UiRole::Selection)))
            .into_any_element()
    }

    /// The folder of a file as rows show it, relative to the project.
    fn location(&self, path: &Path) -> String {
        let parent = path.parent().map(|p| self.rel(p)).unwrap_or_default();
        if parent.is_empty() || parent == "." {
            String::new()
        } else {
            parent
        }
    }

    /// What an empty list says: on All "Nothing found." and a link to
    /// include non-project items; elsewhere the link and Find in Files.
    fn empty_text(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let link = theme.color(UiRole::TextLink);
        let text = self.field.read(cx).text();
        let mut lines: Vec<AnyElement> = Vec::new();
        if self.is_searching() || text.trim().is_empty() {
            return div().into_any_element();
        }
        let scope_link = (!self.everywhere && self.tab != Tab::Actions).then(|| {
            div()
                .id("search-everywhere-set-scope")
                .debug_selector(|| "search-everywhere-set-scope".into())
                .text_color(link)
                .cursor_pointer()
                .child("all places")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_everywhere(true, cx)))
        });
        let row = || div().flex().flex_row().gap(px(4.0));
        match self.tab {
            Tab::All => {
                lines.push(div().child("Nothing found.").into_any_element());
                if let Some(link) = scope_link {
                    lines.push(row().child("Set scope to").child(link).into_any_element());
                }
            }
            Tab::Files => {
                let find = cx
                    .build_action("find-in-files.open", None)
                    .ok()
                    .and_then(|a| self.shortcut(a.as_ref(), window, cx));
                let mut line = row();
                line = match scope_link {
                    Some(link) => line.child("Set scope to").child(link).child("or use"),
                    None => line.child("Use"),
                };
                line = line.child(
                    div()
                        .id("search-everywhere-find-in-files")
                        .text_color(link)
                        .cursor_pointer()
                        .child("Find in Files")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            let text = this.field.read(cx).text();
                            this.close(false, window, cx);
                            cx.emit(SearchEverywhereEvent::FindInFiles(text));
                        })),
                );
                let rest = match find {
                    Some(keys) => format!("({keys}) to perform full-text search."),
                    None => "to perform full-text search.".to_owned(),
                };
                lines.push(line.child(rest).into_any_element());
            }
            Tab::Actions => {}
        }
        div()
            .id("search-everywhere-empty")
            .debug_selector(|| "search-everywhere-empty".into())
            .flex()
            .flex_col()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(px(4.0))
            .text_color(theme.color(UiRole::TextMuted))
            .children(lines)
            .into_any_element()
    }

    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.rows.is_empty() {
            return self.empty_text(window, cx);
        }
        let mut rows = uniform_list(
            "search-everywhere-rows",
            self.rows.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "search-everywhere-rows".into())
        .track_scroll(self.list.clone())
        .flex_1()
        .py(px(4.0));
        rows.style().restrict_scroll_to_axis = Some(true);
        rows.into_any_element()
    }
}

/// The name fragments and the location fragments a Go to File text
/// matches, as the reference highlights them: the name with the longest
/// tail of the name part that matches it, the location with the folder
/// part before that tail.
fn file_highlights(
    text: &str,
    name: &str,
    location: &str,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let sanitized: Vec<char> = sanitized_pattern(text).chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    let start = sanitized
        .iter()
        .rposition(|&c| c == '/')
        .map_or(0, |i| i + 1);
    let last = sanitized.len().saturating_sub(1);
    for i in start..last.max(start + 1) {
        let tail: String = sanitized[i..].iter().collect();
        let matcher = Matcher::new(&format!("*{tail}"), MatchingMode::IgnoreCase);
        if let Some(fragments) = matcher.find_chars(&name_chars) {
            let folder: String = sanitized[..i].iter().collect();
            let folder = folder.trim_end_matches('/');
            let location_ranges = if folder.is_empty() {
                Vec::new()
            } else {
                qualifier_matcher(folder).find(location).unwrap_or_default()
            };
            return (fragments, location_ranges);
        }
    }
    (Vec::new(), Vec::new())
}

/// Char ranges of `text` as byte ranges with a background.
fn highlight(
    text: &str,
    ranges: &[Range<usize>],
    background: gpui::Hsla,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let style = HighlightStyle {
        background_color: Some(background),
        ..HighlightStyle::default()
    };
    ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.min(offsets.len() - 1);
            let b = r.end.min(offsets.len() - 1);
            (b > a).then(|| (offsets[a]..offsets[b], style))
        })
        .collect()
}

impl Render for SearchEverywhere {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let collapsed = self.collapsed(cx);
        // A quarter down the window, as the reference opens it.
        let short = px(HEADER_HEIGHT + FIELD_HEIGHT + 2.0);
        self.frame
            .set_top((window.viewport_size().height / 4.0 - short / 2.0).max(px(16.0)));
        self.frame.set_collapsed(collapsed.then_some(short));
        let content = div()
            .id("search-everywhere")
            .key_context("SearchEverywhere")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            // Clicks inside the popup leave the focus in the field.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(self.render_header(window, cx))
            .child(self.render_field(window, cx))
            .when(!collapsed, |d| d.child(self.render_list(window, cx)));
        let content = Self::bind_actions(content, cx);
        self.frame
            .render(
                "search-everywhere",
                content,
                |this| &mut this.frame,
                window,
                cx,
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // expected fragments
mod tests {
    use super::*;

    #[test]
    fn highlights_of_names_and_folders() {
        let (name, location) = file_highlights("flask/app", "app.py", "src/flask");
        assert_eq!(name, vec![0..3]);
        assert_eq!(location, vec![4..9]);
        let (name, location) = file_highlights("app.py:12", "app.py", "");
        assert_eq!(name, vec![0..6]);
        assert!(location.is_empty());
    }
}
