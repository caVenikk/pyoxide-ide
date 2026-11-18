//! Presentation and persistent choices of the grouped File Structure tree.

use super::*;

/// Long SQL previews retain their first 45 and final five characters.
pub(super) fn shortened_statement(text: &SharedString) -> SharedString {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(45).collect();
    let tail: Vec<char> = chars.rev().take(6).collect();
    if tail.len() < 6 {
        return text.clone();
    }
    format!("{head}…{}", tail[..5].iter().rev().collect::<String>()).into()
}

impl ListPopup {
    pub(super) fn group_enabled(&self, i: usize) -> bool {
        self.options
            .filters
            .get(i)
            .is_none_or(|label| self.structure.groups.get(*label).copied().unwrap_or(i == 0))
    }

    pub(super) fn toggle_group(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(label) = self.options.filters.get(i) {
            self.structure = settings_ui::store::get(cx);
            self.structure
                .groups
                .insert((*label).into(), !self.group_enabled(i));
            settings_ui::store::set(cx, &self.structure);
            self.keep_or_select_first();
            cx.notify();
        }
    }

    fn keep_or_select_first(&mut self) {
        let shown = self.shown();
        self.selected = self
            .selected
            .filter(|&i| shown[i])
            .or_else(|| self.first_shown());
    }

    pub(super) fn sort_rows(&mut self) {
        if !self.grouped() {
            return;
        }
        if !self.structure.sort_alphabetically {
            self.order.sort_unstable();
            return;
        }
        // A path of sibling names and stable source indices keeps a parent
        // together with its descendants, even when sibling names are equal.
        let mut path = Vec::new();
        let keys: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                if let Some(row) = item.row() {
                    path.truncate(usize::from(row.indent));
                    path.push((row.text.to_lowercase(), i));
                }
                path.clone()
            })
            .collect();
        self.order.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
    }

    pub(super) fn open_structure_settings(
        &mut self,
        _: &crate::commands::StructureSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.grouped() || self.menu.is_some() {
            return;
        }
        use ui::menu::{ContextMenu, MenuItem};
        let checked = |item: MenuItem, enabled| {
            if enabled {
                item.icon(IconName::CHECKED)
            } else {
                item
            }
        };
        let items = vec![
            checked(
                MenuItem::action("Sort Alphabetically", crate::commands::SortStructure),
                self.structure.sort_alphabetically,
            ),
            MenuItem::Separator,
            checked(
                MenuItem::action("Narrow Down on Typing", crate::commands::NarrowStructure),
                self.structure.narrow_down,
            ),
        ];
        let bounds = self.frame.bounds(self.frame.viewport(window));
        let origin = if ui::popup_window::displays(cx.entity_id(), window, cx) {
            gpui::point(px(0.0), px(0.0))
        } else {
            bounds.origin
        };
        let at = origin + gpui::point(bounds.size.width - px(27.0), px(72.0));
        let menu = ContextMenu::build(items, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, at, subscription));
        cx.notify();
    }

    pub(super) fn sort_structure(
        &mut self,
        _: &crate::commands::SortStructure,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.grouped() {
            return;
        }
        self.structure = settings_ui::store::get(cx);
        self.structure.sort_alphabetically = !self.structure.sort_alphabetically;
        settings_ui::store::set(cx, &self.structure);
        self.sort_rows();
        cx.notify();
    }

    pub(super) fn narrow_structure(
        &mut self,
        _: &crate::commands::NarrowStructure,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.grouped() {
            return;
        }
        self.structure = settings_ui::store::get(cx);
        self.structure.narrow_down = !self.structure.narrow_down;
        settings_ui::store::set(cx, &self.structure);
        self.keep_or_select_first();
        cx.notify();
    }

    pub(super) fn is_branch(&self, index: usize) -> bool {
        self.grouped()
            && self
                .items
                .get(index)
                .and_then(ListItem::row)
                .zip(self.items.get(index + 1).and_then(ListItem::row))
                .is_some_and(|(row, next)| next.indent > row.indent)
    }

    pub(super) fn toggle_branch(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&index) {
            self.collapsed.insert(index);
        }
        self.selected = Some(index);
        cx.notify();
    }

    pub(super) fn filter_controls(&self, cx: &Context<Self>) -> gpui::Div {
        let line = cx.theme().color(UiRole::PopupBorder);
        let control = |i: usize, label: &&str| {
            let shortcut = ["D", "M", "S", "O"].get(i).copied().unwrap_or("");
            let modifier = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            ui::form::check_row(
                ("structure-group", i),
                ("structure-check", i),
                self.group_enabled(i),
                format!("{label} ({modifier}{shortcut})"),
                cx,
            )
            .w(px(195.0))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_group(i, cx);
                window.focus(&this.focus);
            }))
        };
        div()
            .flex()
            .flex_col()
            .w_full()
            .pr(px(48.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(line)
            .px(px(10.0))
            .gap(px(4.0))
            .children(
                self.options
                    .filters
                    .chunks(2)
                    .enumerate()
                    .map(|(row, labels)| {
                        div().flex().flex_row().gap(px(4.0)).children(
                            labels
                                .iter()
                                .enumerate()
                                .map(|(col, label)| control(2 * row + col, label)),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statement_previews_keep_the_end_after_a_unicode_safe_ellipsis() {
        assert_eq!(
            shortened_statement(&"INSERT INTO review_filter_480 (sample_id) VALUES (1)".into())
                .as_ref(),
            "INSERT INTO review_filter_480 (sample_id) VAL…S (1)"
        );
        let unicode = format!("{}абвгдеёж", "界".repeat(44));
        assert_eq!(
            shortened_statement(&unicode.into()).as_ref(),
            format!("{}а…гдеёж", "界".repeat(44))
        );
    }
}
