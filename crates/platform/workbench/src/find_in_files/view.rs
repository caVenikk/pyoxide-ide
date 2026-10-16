//! Drawing the Find in Files popup: the header (title, counter, loading
//! indicator, file mask, pin), the search and replace fields, the scope
//! row, the results over the preview with a splitter between them, and the
//! bottom row (the hint for queries over several lines, Replace All and
//! Replace).

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, FontWeight, HighlightStyle, HitboxBehavior,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Render,
    SharedString, StyledText, Window, anchored, canvas, deferred, div, point, px, uniform_list,
};
use themes::{Metric, UiRole};
use ui::components::{
    SMALL_BUTTON, Tooltip, checkbox, input_frame, small_icon_button, text_button, toggle_button,
};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::dropdown::DropKind;
use super::*;

/// Height of the preview's title row.
const PREVIEW_TITLE: f32 = 28.0;
/// Longest path in the preview's title before it is shortened in the
/// middle.
const TITLE_PATH_CHARS: usize = 120;
/// Height of the bottom row.
const BOTTOM_HEIGHT: f32 = 40.0;
/// Width of the module and scope lists (the reference's 300 px).
const COMBO_WIDTH: f32 = 300.0;

impl FindInFiles {
    fn shortcut(
        &self,
        action: &dyn gpui::Action,
        window: &Window,
        cx: &App,
    ) -> Option<SharedString> {
        let focus = self.query.focus_handle(cx);
        ui::keys::shortcut_in(action, &focus, window).map(SharedString::from)
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let masks = self.render_dropdown(&[DropKind::Masks], cx);
        let theme = cx.theme();
        let mask_focused = self.mask.focus_handle(cx).is_focused(window);
        let mask_on = self.mask_on;
        let mask_field =
            input_frame(mask_focused, self.invalid_field == Some(Field::Mask), cx)
                .id("find-in-files-mask")
                .debug_selector(|| "find-in-files-mask".into())
                .on_mouse_down(MouseButton::Left, crate::popup::no_move)
                .key_context("FindInFilesMask")
                .w(px(MASK_WIDTH))
                .h(px(SMALL_BUTTON + 4.0))
                .pl(px(6.0))
                .when(!mask_on, |d| d.opacity(0.5))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .h(editor::field_line_height(window, cx))
                        .overflow_hidden()
                        .child(self.mask.clone()),
                )
                .child(
                    small_icon_button("find-in-files-masks", IconName::CHEVRON_DOWN, mask_on, cx)
                        .when(mask_on, |d| {
                            d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.show_dropdown(DropKind::Masks, window, cx)
                            }))
                        }),
                )
                // Off, the mask cannot be edited (as the reference's disabled
                // combo).
                .when(!mask_on, |d| {
                    d.child(
                        div()
                            .id("find-in-files-mask-off")
                            .absolute()
                            .size_full()
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            }),
                    )
                })
                .children(masks);
        let title = if self.replace_mode {
            "Replace in Files"
        } else {
            "Find in Files"
        };
        div()
            .id("find-in-files-header")
            .debug_selector(|| "find-in-files-header".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(8.0))
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
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id("find-in-files-info")
                    .debug_selector(|| "find-in-files-info".into())
                    .flex_none()
                    .pl(px(4.0))
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(self.info()),
            )
            .when(self.is_running(), |d| {
                d.child(ui::components::spinner("find-in-files-loading", cx))
            })
            .child(div().flex_1())
            .child(
                div()
                    .id("find-in-files-mask-toggle")
                    .debug_selector(|| "find-in-files-mask-toggle".into())
                    .on_mouse_down(MouseButton::Left, crate::popup::no_move)
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .child(checkbox("find-in-files-mask-box", mask_on, cx))
                    // A check box shows its mnemonic while Option (Alt) is
                    // held, as the reference's do; the scope tabs always.
                    .child(if window.modifiers().alt {
                        mnemonic_label("File mask:", 8)
                    } else {
                        StyledText::new("File mask:")
                    })
                    .tooltip(Tooltip::text(
                        "File mask",
                        self.shortcut(&ToggleMask, window, cx),
                    ))
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.toggle_mask(window, cx)
                        }),
                    ),
            )
            .child(mask_field)
            .child(
                toggle_button("find-in-files-pin", IconName::PIN, self.pinned, cx)
                    .debug_selector(|| "find-in-files-pin".into())
                    .on_mouse_down(MouseButton::Left, crate::popup::no_move)
                    .tooltip(Tooltip::text(
                        "Pin Window",
                        self.shortcut(&TogglePin, window, cx),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_pin(cx))),
            )
    }

    /// The search or the replace field: the history icon, the text and its
    /// buttons; it grows with the lines of its text.
    fn render_text_field(
        &self,
        field: Field,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (editor, id, context, history) = match field {
            Field::Replace => (
                self.replace.clone(),
                "find-in-files-replace",
                "FindInFilesReplace",
                DropKind::ReplaceHistory,
            ),
            _ => (
                self.query.clone(),
                "find-in-files-query",
                "FindInFilesQuery",
                DropKind::SearchHistory,
            ),
        };
        let line = editor::field_line_height(window, cx);
        let rows = editor
            .read(cx)
            .buffer()
            .len_lines()
            .clamp(1, editor::find::MAX_FIELD_ROWS);
        let pad = ((px(FIELD_HEIGHT) - line) / 2.0).max(px(0.0));
        let height = px(FIELD_HEIGHT) + line * (rows - 1) as f32;
        let error = editor.read(cx).is_error();
        let new_line = {
            let target = editor.clone();
            small_icon_button(
                SharedString::from(format!("{id}-new-line")),
                IconName::NEW_LINE,
                true,
                cx,
            )
            .tooltip(Tooltip::text(
                "New Line",
                self.shortcut(&editor::find::NewLine, window, cx),
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&target.focus_handle(cx));
                target.update(cx, |f, cx| f.insert("\n", cx));
            }))
        };
        let options = self.options;
        let buttons = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(FIELD_HEIGHT))
            .gap(px(2.0))
            .child(new_line)
            .when(field == Field::Query, |d| {
                d.child(
                    toggle_button(
                        "find-in-files-match-case",
                        IconName::MATCH_CASE,
                        options.case_sensitive,
                        cx,
                    )
                    .tooltip(Tooltip::text(
                        "Match Case",
                        self.shortcut(&editor::find::ToggleCase, window, cx),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_option(|o| o.case_sensitive = !o.case_sensitive, cx)
                    })),
                )
                .child(
                    toggle_button(
                        "find-in-files-words",
                        IconName::WORDS,
                        options.whole_words,
                        cx,
                    )
                    .tooltip(Tooltip::text(
                        "Words",
                        self.shortcut(&editor::find::ToggleWords, window, cx),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_option(|o| o.whole_words = !o.whole_words, cx)
                    })),
                )
                .child(
                    toggle_button("find-in-files-regex", IconName::REGEX, options.regex, cx)
                        .tooltip(Tooltip::text(
                            "Regex",
                            self.shortcut(&editor::find::ToggleRegex, window, cx),
                        ))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_option(|o| o.regex = !o.regex, cx)
                        })),
                )
            });
        div()
            .id(id)
            .debug_selector(move || id.into())
            .key_context(context)
            .relative()
            .flex()
            .flex_row()
            .flex_none()
            .items_start()
            .h(height)
            .px(px(PAD - 4.0))
            .gap(px(4.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            .when(error, |d| d.bg(theme.color(UiRole::DangerBg)))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-history")))
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(FIELD_HEIGHT))
                    .px(px(4.0))
                    .child(Icon::new(IconName::SEARCH_HISTORY))
                    .tooltip(Tooltip::text(
                        if field == Field::Replace {
                            "Replace History"
                        } else {
                            "Search History"
                        },
                        self.shortcut(&editor::find::ShowHistory, window, cx),
                    ))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.show_dropdown(history, window, cx)
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .mt(pad)
                    .h(line * rows as f32)
                    .overflow_hidden()
                    .child(editor),
            )
            .child(buttons)
            .children(self.render_dropdown(&[history], cx))
    }

    fn scope_tab(
        &self,
        id: &'static str,
        (label, mnemonic): (&'static str, usize),
        kind: ScopeKind,
        action: &dyn gpui::Action,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let selected = self.scope == kind;
        let hover = theme.color(UiRole::Hover);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .flex_none()
            .items_center()
            .h(px(SMALL_BUTTON + 2.0))
            .px(px(8.0))
            .rounded(theme.metric(Metric::CornerRadius))
            .when(selected, |d| {
                d.bg(theme.color(UiRole::SearchOptionSelected))
            })
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .child(mnemonic_label(label, mnemonic))
            .tooltip(Tooltip::text(label, self.shortcut(action, window, cx)))
            .on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_scope(kind, window, cx)
                }),
            )
    }

    /// The scope tabs and what the selected one needs: the folder for
    /// Directory, the scope list for Scope.
    /// A list field (the module or the named scope): its text and the
    /// arrow; a click opens its list.
    fn combo(
        &self,
        id: &'static str,
        text: String,
        kind: DropKind,
        list: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .debug_selector(move || id.into())
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(px(SMALL_BUTTON + 4.0))
            .px(px(8.0))
            .rounded(theme.metric(Metric::CornerRadius) * 1.5)
            .border_1()
            .border_color(theme.color(UiRole::BorderInput))
            // The reference's module list is as wide as its scope list.
            .min_w(px(COMBO_WIDTH))
            .child(div().flex_1().child(text))
            .child(Icon::new(IconName::CHEVRON_DOWN))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.show_dropdown(kind, window, cx)
            }))
            .children(list)
            .into_any_element()
    }

    fn render_scopes(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let directories = self.render_dropdown(&[DropKind::Directories], cx);
        let scopes = self.render_dropdown(&[DropKind::Scopes], cx);
        let modules = self.render_dropdown(&[DropKind::Modules], cx);
        let module = self.combo(
            "find-in-files-module-list",
            self.module_name(),
            DropKind::Modules,
            modules,
            cx,
        );
        let named = self.combo(
            "find-in-files-scope-list",
            self.named.name().to_owned(),
            DropKind::Scopes,
            scopes,
            cx,
        );
        let theme = cx.theme();
        let details: Option<AnyElement> = match self.scope {
            ScopeKind::Project => None,
            ScopeKind::Module => Some(module),
            ScopeKind::Directory => {
                let focused = self.directory.focus_handle(cx).is_focused(window);
                let bad = self.invalid_field == Some(Field::Directory);
                Some(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .items_center()
                        .gap(px(4.0))
                        .child(
                            input_frame(focused, bad, cx)
                                .id("find-in-files-directory")
                                .debug_selector(|| "find-in-files-directory".into())
                                .key_context("FindInFilesDirectory")
                                .flex_1()
                                .min_w(px(200.0))
                                .h(px(SMALL_BUTTON + 4.0))
                                .pl(px(6.0))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .h(editor::field_line_height(window, cx))
                                        .overflow_hidden()
                                        .child(self.directory.clone()),
                                )
                                .child(
                                    small_icon_button(
                                        "find-in-files-directories",
                                        IconName::CHEVRON_DOWN,
                                        true,
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        |this, _: &ClickEvent, window, cx| {
                                            this.show_dropdown(DropKind::Directories, window, cx)
                                        },
                                    )),
                                )
                                .children(directories),
                        )
                        .child(
                            small_icon_button(
                                "find-in-files-browse",
                                IconName::MORE_HORIZONTAL,
                                true,
                                cx,
                            )
                            .tooltip(Tooltip::text(
                                "Browse",
                                self.shortcut(&BrowseDirectory, window, cx),
                            ))
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    this.browse_directory(window, cx)
                                },
                            )),
                        )
                        .child(
                            toggle_button(
                                "find-in-files-recursive",
                                IconName::SHOW_AS_TREE,
                                self.recursive,
                                cx,
                            )
                            .tooltip(Tooltip::text(
                                "Search recursively in subdirectories",
                                self.shortcut(&ToggleRecursive, window, cx),
                            ))
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| {
                                    this.toggle_recursive_action(&ToggleRecursive, window, cx)
                                },
                            )),
                        )
                        .into_any_element(),
                )
            }
            ScopeKind::Named => Some(named),
        };
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(4.0))
            .h(px(SCOPE_HEIGHT))
            .px(px(PAD - 4.0))
            .border_b_1()
            .border_color(theme.color(UiRole::Divider))
            // The reference's labels and mnemonics: In &Project, &Module,
            // &Directory, &Scope.
            .child(self.scope_tab(
                "find-in-files-scope-project",
                ("In Project", 3),
                ScopeKind::Project,
                &ScopeProject,
                window,
                cx,
            ))
            .child(self.scope_tab(
                "find-in-files-scope-module",
                ("Module", 0),
                ScopeKind::Module,
                &ScopeModule,
                window,
                cx,
            ))
            .child(self.scope_tab(
                "find-in-files-scope-directory",
                ("Directory", 0),
                ScopeKind::Directory,
                &ScopeDirectory,
                window,
                cx,
            ))
            .child(self.scope_tab(
                "find-in-files-scope-named",
                ("Scope", 0),
                ScopeKind::Named,
                &ScopeNamed,
                window,
                cx,
            ))
            .when_some(details, |d, details| {
                d.child(div().w(px(12.0))).child(details)
            })
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let Some(row) = self.results.row(index) else {
            return div().into_any_element();
        };
        let selected = self.selected == Some(index);
        let (text, highlights) = row_text(row, theme.color(UiRole::SearchMatch));
        let file_name = row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let location = format!("{file_name} {}", row.line.line + 1);
        div()
            .id(("find-in-files-row", index))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(row_height)
            .px(px(PAD))
            .gap(px(12.0))
            .when(selected, |d| d.bg(theme.color(UiRole::Selection)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    // The editor font, with ligatures as the editor has them.
                    .font(gpui::Font {
                        family: theme.mono_family(),
                        features: if theme.raw().fonts.editor_ligatures {
                            gpui::FontFeatures::default()
                        } else {
                            gpui::FontFeatures::disable_ligatures()
                        },
                        fallbacks: None,
                        weight: FontWeight::NORMAL,
                        style: gpui::FontStyle::Normal,
                    })
                    .child(StyledText::new(text).with_highlights(highlights)),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(location),
            )
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                this.select(index, cx);
                if e.click_count() >= 2 {
                    this.open_result(window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.results.len();
        let mut rows = uniform_list(
            "find-in-files-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|i| this.render_row(i, cx)).collect::<Vec<_>>()
            }),
        )
        .debug_selector(|| "find-in-files-rows".into())
        .track_scroll(self.list.clone())
        .flex_1()
        .py(px(4.0))
        .overflow_x_hidden();
        rows.style().restrict_scroll_to_axis = Some(true);
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(rows)
            // Loading more: a spinner at the bottom, as the reference's
            // loading row.
            .when(self.loading_more, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .py(px(4.0))
                        .child(ui::components::spinner("find-in-files-loading-more", cx)),
                )
            })
            .into_any_element()
    }

    /// The preview's title: the file name, then its folder in the project
    /// (shortened in the middle when long), muted.
    fn render_preview_title(&self, cx: &App) -> Option<AnyElement> {
        let path = self.preview.path(cx)?;
        let theme = cx.theme();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = path
            .parent()
            .map(|p| {
                let rel = self.rel(p);
                if rel.is_empty() {
                    String::new()
                } else {
                    shorten_middle(&rel, TITLE_PATH_CHARS)
                }
            })
            .unwrap_or_default();
        Some(
            div()
                .id("find-in-files-preview-title")
                .debug_selector(|| "find-in-files-preview-title".into())
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .h(px(PREVIEW_TITLE))
                .px(px(PAD))
                .whitespace_nowrap()
                .overflow_hidden()
                .child(name)
                .child(
                    div()
                        .text_color(theme.color(UiRole::TextMuted))
                        .child(folder),
                )
                .into_any_element(),
        )
    }

    /// The results over the preview, with the splitter between them; only
    /// the message when there are no results.
    fn render_split(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if let Some(text) = self.empty_text() {
            return div()
                .id("find-in-files-empty")
                .debug_selector(|| "find-in-files-empty".into())
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(text)
                .into_any_element();
        }
        let this = cx.entity();
        let measure = canvas(
            move |bounds, _, cx| {
                this.update(cx, |this, _| this.split_bounds = Some(bounds));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let divider = theme.color(UiRole::Divider);
        let splitter = div()
            .id("find-in-files-splitter")
            .debug_selector(|| "find-in-files-splitter".into())
            .relative()
            .flex_none()
            .h(px(1.0))
            .bg(divider)
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
                        cx.listener(|this, e: &MouseDownEvent, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.split_drag.start((e.position.y, this.split));
                            cx.notify();
                        }),
                    ),
            );
        let tracking = crate::popup::track_held_drag(
            cx.entity(),
            |this: &mut Self| &mut this.split_drag,
            |this, mouse, cx| this.drag_split(mouse.y, cx),
            |_, _| {},
            self.split_drag
                .is_dragging()
                .then_some(CursorStyle::ResizeUpDown),
        );
        let list_share = self.split;
        div()
            .id("find-in-files-split")
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
                    .flex_basis(gpui::relative(list_share))
                    .min_h(px(48.0))
                    .child(self.render_list(cx)),
            )
            .child(splitter)
            .child(
                div()
                    .id("find-in-files-preview")
                    .debug_selector(|| "find-in-files-preview".into())
                    .flex()
                    .flex_col()
                    .flex_grow()
                    .flex_basis(gpui::relative(1.0 - list_share))
                    .min_h(px(48.0))
                    .overflow_hidden()
                    .children(self.render_preview_title(cx))
                    .child(
                        div()
                            .key_context("FindInFilesPreview")
                            .flex_1()
                            .min_h(px(0.0))
                            .child(self.preview.view().clone())
                            .children(self.render_preview_hint(cx)),
                    ),
            )
            .child(tracking)
            .into_any_element()
    }

    /// The hint of Find Next in the preview ("not found, press ... to
    /// search from the top") below the caret, as in any editor.
    fn render_preview_hint(&self, cx: &App) -> Option<AnyElement> {
        let bar = self.preview.find_bar().read(cx);
        let (hint, above) = (bar.hint()?.clone(), bar.hint_above());
        let caret = self.preview.editor().read(cx).caret_bounds()?;
        let theme = cx.theme();
        let at = if above {
            point(caret.left(), caret.top() - px(2.0))
        } else {
            point(caret.left(), caret.bottom() + px(2.0))
        };
        let anchor = if above {
            gpui::Corner::BottomLeft
        } else {
            gpui::Corner::TopLeft
        };
        Some(
            deferred(
                anchored().position(at).anchor(anchor).child(
                    div()
                        .id("find-in-files-preview-hint")
                        .debug_selector(|| "find-in-files-preview-hint".into())
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(theme.metric(Metric::CornerRadius))
                        .border_1()
                        .border_color(theme.color(UiRole::PopupBorder))
                        .bg(theme.color(UiRole::SurfaceRaised))
                        .shadow(ui::components::popup_shadow())
                        .whitespace_nowrap()
                        .child(hint),
                ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn drag_split(&mut self, y: gpui::Pixels, cx: &mut Context<Self>) {
        let (Some((start, share)), Some(bounds)) = (self.split_drag.get(), self.split_bounds)
        else {
            return;
        };
        let height = f32::from(bounds.size.height).max(1.0);
        let moved = f32::from(y - start) / height;
        self.split = (share + moved).clamp(0.1, 0.9);
        cx.notify();
    }

    /// The bottom row: the hint for queries over several lines, and Replace
    /// All and Replace in Replace in Files.
    fn render_bottom(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let multiline = self.query.read(cx).buffer().len_lines() > 1;
        if !multiline && !self.replace_mode {
            return None;
        }
        let theme = cx.theme();
        let hint = multiline
            .then(|| {
                let next = self.shortcut(&NextResult, window, cx)?;
                let previous = self.shortcut(&PreviousResult, window, cx)?;
                Some(format!("Use {next} and {previous} to select usages"))
            })
            .flatten();
        let ready = !self.stale && !self.results.is_empty();
        let replace_buttons =
            self.replace_mode.then(|| {
                let all = text_button("find-in-files-replace-all", "Replace All", ready, cx)
                    .tooltip(Tooltip::text(
                        "Replace All",
                        self.shortcut(&ReplaceAll, window, cx),
                    ))
                    .when(ready, |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.replace_all(window, cx)
                        }))
                    });
                let one = text_button(
                    "find-in-files-replace-selected",
                    "Replace",
                    ready && self.selected.is_some(),
                    cx,
                )
                .when(ready, |d| {
                    d.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.replace_selected(window, cx)
                    }))
                });
                div().flex().flex_row().gap(px(8.0)).child(all).child(one)
            });
        Some(
            div()
                .id("find-in-files-bottom")
                .debug_selector(|| "find-in-files-bottom".into())
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .h(px(BOTTOM_HEIGHT))
                .px(px(PAD))
                .border_t_1()
                .border_color(theme.color(UiRole::Divider))
                .bg(theme.color(UiRole::SurfaceAlt))
                .children(hint.map(|h| {
                    div()
                        .text_size(theme.ui_size() * 0.9)
                        .text_color(theme.color(UiRole::TextDisabled))
                        .child(h)
                }))
                .child(div().flex_1())
                .children(replace_buttons)
                .into_any_element(),
        )
    }
}

/// A label with its mnemonic letter (the char at `index`) underlined, as
/// the reference draws its mnemonics.
fn mnemonic_label(label: &'static str, index: usize) -> StyledText {
    let start = label.char_indices().nth(index).map_or(0, |(i, _)| i);
    let end = label[start..]
        .chars()
        .next()
        .map_or(start, |c| start + c.len_utf8());
    let underline = HighlightStyle {
        underline: Some(gpui::UnderlineStyle {
            thickness: px(1.0),
            color: None,
            wavy: false,
        }),
        ..HighlightStyle::default()
    };
    StyledText::new(label).with_highlights([(start..end, underline)])
}

/// A path shortened in the middle to at most `max` chars.
fn shorten_middle(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// A row's text without its indentation (tabs as spaces), and the matches
/// as byte ranges of it with their background.
fn row_text(
    row: &project_search::Row,
    background: gpui::Hsla,
) -> (String, Vec<(std::ops::Range<usize>, HighlightStyle)>) {
    let chars: Vec<char> = row.line.text.chars().collect();
    let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
    let text: String = chars[indent..]
        .iter()
        .map(|&c| if c == '\t' { ' ' } else { c })
        .collect();
    // Byte offset of each char of `text`, and the end.
    let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    offsets.push(text.len());
    let start = row.line.text_column + indent;
    let style = HighlightStyle {
        background_color: Some(background),
        ..HighlightStyle::default()
    };
    let highlights = row
        .line
        .ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.saturating_sub(start).min(offsets.len() - 1);
            let b = r.end.saturating_sub(start).min(offsets.len() - 1);
            (b > a).then(|| (offsets[a]..offsets[b], style))
        })
        .collect();
    (text, highlights)
}

impl Render for FindInFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let context = if self.replace_mode {
            "FindInFiles ReplaceInFiles"
        } else {
            "FindInFiles"
        };
        let content = div()
            .id("find-in-files")
            .key_context(context)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            // Clicks inside the popup leave the focus in its fields.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            // The check box's mnemonic shows while Option (Alt) is held.
            .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
            .child(self.render_header(window, cx))
            .child(self.render_text_field(Field::Query, window, cx))
            .when(self.replace_mode, |d| {
                d.child(self.render_text_field(Field::Replace, window, cx))
            })
            .child(self.render_scopes(window, cx))
            .child(self.render_split(cx))
            .children(self.render_bottom(window, cx));
        let content = Self::bind_actions(content, cx);
        if self.wants_more() {
            cx.defer_in(window, |this, _, cx| this.load_more(cx));
        }
        self.frame
            .render("find-in-files", content, |this| &mut this.frame, window, cx)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_drops_the_indentation_and_maps_matches_to_bytes() {
        let row = project_search::Row {
            path: std::sync::Arc::new(PathBuf::from("a.py")),
            line: project_search::LineMatch {
                line: 0,
                text: "\t  ä = \"ä\"".into(),
                text_column: 0,
                ranges: vec![3..4, 8..9],
                first: Point::new(0, 3)..Point::new(0, 4),
            },
        };
        let (text, highlights) = row_text(&row, gpui::Hsla::default());
        assert_eq!(text, "ä = \"ä\"");
        let ranges: Vec<_> = highlights.into_iter().map(|(r, _)| r).collect();
        assert_eq!(ranges, vec![0..2, 6..8]);
    }

    #[test]
    fn long_paths_are_shortened_in_the_middle() {
        assert_eq!(shorten_middle("src/app", 120), "src/app");
        let long = "a".repeat(200);
        let short = shorten_middle(&long, 120);
        assert_eq!(short.chars().count(), 120);
        assert!(short.contains('…'));
    }
}
