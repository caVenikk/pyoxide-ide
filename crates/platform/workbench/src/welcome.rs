//! The Welcome tab (the reference's non-modal Welcome screen, its right
//! tab: `WelcomeScreenRightTab` of `intellij.platform.ide.nonModalWelcomeScreen`,
//! IntelliJ Community, Apache-2.0). An editor tab of a main window without
//! a project: "Welcome to <product>" over "Start in one click", the tiles
//! of what to start with (the product's, then Open File), and at the
//! bottom the Theme and Keymap combo boxes, whose lists switch the theme
//! and the keymap at once (`settings.switch-theme`, `settings.switch-keymap`).
//! A new person chooses the theme and the keymap there: the first start
//! has no pages of its own for them.
//!
//! Metrics of the reference: the title 20 (line 24) and 8 above the
//! subtitle 13 (line 16), 32 to the tiles; tiles of 128 by 96 with corners
//! of 8, 16 apart, three in a row, an icon of 24 over the label 8 below
//! it; their background white at 7 % (dark themes) or black at 5 %, 12 %
//! or 8 % under the mouse, 18 % or 12 % pressed. The combo boxes are 16
//! apart, 12 above the bottom; their lists are at least 200 wide.

use gpui::{
    AnyElement, App, BoxShadow, ClickEvent, Context, Corner, DismissEvent, Entity, FocusHandle,
    Focusable, Hsla, IntoElement, MouseButton, Render, SharedString, Subscription, Window,
    anchored, deferred, div, point, prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::commands::SwitchTheme;
use settings_ui::theme_switch;
use themes::UiRole;
use ui::components::Tooltip;
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;

pub mod projects;

use crate::keymap::active::{self as keymaps, SwitchKeymap};
use crate::tab_view::TabView;

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// The Welcome tab.
    OpenWelcome => "welcome.open", "Welcome",
        "Opens the Welcome tab in the editor area (selects it when it is open): the tiles to start with and the Theme and Keymap combo boxes.";
    /// The Theme list of the Welcome tab.
    ChooseTheme => "welcome.theme", "Theme",
        "Opens the list of the Welcome tab's Theme combo box (the tab opens first if it is not); a theme chosen in it is the theme at once.";
    /// The Keymap list of the Welcome tab.
    ChooseKeymap => "welcome.keymap", "Keymap",
        "Opens the list of the Welcome tab's Keymap combo box (the tab opens first if it is not); a keymap chosen in it is the keymap at once.";
    /// A new script of the product's language.
    NewScript { extension: Option<String>, directory: Option<String> } => "welcome.new-script", "New Script",
        "Creates an empty file `scratch.<extension>` (then `scratch_1.<extension>`, ...; `txt` without an extension) in `scratches` of the configuration folder, or in `directory`, and opens it in a tab.",
        r#"{"extension": "py"}"#;
}

/// `non.modal.welcome.screen.secondary.title` (`PyWelcomeBundle`).
pub const SUBTITLE: &str = "Start in one click";
/// `welcome.screen.right.tab.theme.switch.prefix` and
/// `welcome.screen.right.tab.keymap.switch.prefix`
/// (`NonModalWelcomeScreenBundle`).
pub const THEME_PREFIX: &str = "Theme: ";
pub const KEYMAP_PREFIX: &str = "Keymap: ";
/// `non.modal.welcome.screen.feature.open.file`, `.learn`, `.plugins`.
pub const OPEN_FILE: &str = "Open File";
pub const LEARN: &str = "Learn";
pub const PLUGINS: &str = "Plugins";
/// The tooltip of a tile with nothing behind it yet.
pub const NOT_AVAILABLE: &str = projects::NOT_AVAILABLE;

const TITLE_SIZE: f32 = 20.0;
const TITLE_LINE: f32 = 24.0;
const SUBTITLE_SIZE: f32 = 13.0;
const SUBTITLE_LINE: f32 = 16.0;
const TILE_WIDTH: f32 = 128.0;
const TILE_HEIGHT: f32 = 96.0;
const TILE_GAP: f32 = 16.0;
const TILE_RADIUS: f32 = 8.0;
const TILE_ICON: f32 = 24.0;
const TILES_PER_ROW: usize = 3;
const COMBO_HEIGHT: f32 = 24.0;
const COMBO_GAP: f32 = 16.0;
const FOOTER_BOTTOM: f32 = 12.0;
const LIST_MIN_WIDTH: f32 = 200.0;
/// The lists' rows (Jewel's `SimpleListItem` in the popup of a
/// `ListComboBox`), their selection grey (`List.selectionInactiveBackground`).
const LIST_ROW: f32 = 22.0;
/// The focus ring of a tile (Jewel's `focusOutline`).
const FOCUS_WIDTH: f32 = 2.0;
/// The glow's blobs.
const GLOW_SIZE: f32 = 560.0;

/// How a tile's icon is drawn: in the accent colour (the reference's
/// blue line icons), or in its own colours (a product's logo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileTint {
    Accent,
    Own,
}

/// A tile of the Welcome tab: a command of the registry and its arguments
/// (JSON); without a command it has nothing behind it yet and is drawn
/// dimmed, with [`NOT_AVAILABLE`] as its tooltip.
#[derive(Debug, Clone, Copy)]
pub struct WelcomeTile {
    pub label: &'static str,
    pub icon: IconName,
    pub tint: TileTint,
    pub command: Option<&'static str>,
    pub args: Option<&'static str>,
}

impl WelcomeTile {
    /// A tile with nothing behind it yet.
    pub const fn unavailable(label: &'static str, icon: IconName, tint: TileTint) -> Self {
        Self {
            label,
            icon,
            tint,
            command: None,
            args: None,
        }
    }
}

/// What a product adds to the Welcome tab.
#[derive(Debug, Clone, Default)]
pub struct WelcomeHook {
    /// The tab's icon (the product's); a text file's without one.
    pub icon: Option<IconName>,
    /// The product's tiles, before the platform's.
    pub tiles: Vec<WelcomeTile>,
}

/// The platform's tiles after the product's, in the reference's order:
/// Open File (`IMPORT_FILE`, the `OpenFile` action: a file chooser), Learn
/// (the Learn tab in the editor) and Plugins (Settings > Plugins); we have
/// neither of the last two yet.
fn platform_tiles() -> Vec<WelcomeTile> {
    vec![
        WelcomeTile {
            label: OPEN_FILE,
            icon: IconName::UPLOAD,
            tint: TileTint::Accent,
            command: Some("project.open-file"),
            args: None,
        },
        WelcomeTile::unavailable(LEARN, IconName::LEARN, TileTint::Accent),
        WelcomeTile::unavailable(PLUGINS, IconName::PLUGIN, TileTint::Accent),
    ]
}

/// The two combo boxes at the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combo {
    Theme,
    Keymap,
}

impl Combo {
    fn name(self) -> &'static str {
        match self {
            Combo::Theme => "theme",
            Combo::Keymap => "keymap",
        }
    }

    fn ix(self) -> usize {
        match self {
            Combo::Theme => 0,
            Combo::Keymap => 1,
        }
    }
}

/// An open list of a combo box.
struct OpenList {
    combo: Combo,
    menu: Entity<ContextMenu>,
    _subscription: Subscription,
}

pub struct WelcomeView {
    focus: FocusHandle,
    title: SharedString,
    icon: IconName,
    tiles: Vec<WelcomeTile>,
    /// The tiles' focus: the first one has it when the tab is selected
    /// (the reference requests the focus into the tab when it shows).
    tile_focus: Vec<FocusHandle>,
    /// The tile that had the focus when the tab was last drawn (for the
    /// state).
    focused_tile: Option<usize>,
    list: Option<OpenList>,
}

impl WelcomeView {
    pub fn new(product: &str, hook: &WelcomeHook, cx: &mut Context<Self>) -> Self {
        let mut tiles = hook.tiles.clone();
        tiles.extend(platform_tiles());
        let tile_focus = tiles.iter().map(|_| cx.focus_handle()).collect();
        Self {
            focus: cx.focus_handle(),
            title: format!("Welcome to {product}").into(),
            icon: hook.icon.unwrap_or(IconName::FILE_TEXT),
            tiles,
            tile_focus,
            focused_tile: None,
            list: None,
        }
    }

    /// The theme's name as the combo box shows it.
    fn theme_name(cx: &App) -> String {
        let id = theme_switch::current_theme(cx);
        theme_switch::catalog(cx)
            .get(&id)
            .map(|e| e.name.clone())
            .unwrap_or(id)
    }

    /// The rows of a list and the one in use.
    fn rows(combo: Combo, cx: &App) -> (Vec<MenuItem>, Option<usize>) {
        match combo {
            // Every theme shown, without groups (`ThemeListProvider`).
            Combo::Theme => {
                let current = theme_switch::current_theme(cx);
                let mut selected = None;
                let items = theme_switch::catalog(cx)
                    .themes()
                    .enumerate()
                    .map(|(ix, e)| {
                        if e.id == current {
                            selected = Some(ix);
                        }
                        MenuItem::action(
                            e.name.clone(),
                            SwitchTheme {
                                id: Some(e.id.clone()),
                            },
                        )
                    })
                    .collect();
                (items, selected)
            }
            Combo::Keymap => {
                let current = keymaps::active_name(cx);
                let mut selected = None;
                let items = keymaps::entries(cx)
                    .into_iter()
                    .enumerate()
                    .map(|(ix, e)| {
                        if e.name == current {
                            selected = Some(ix);
                        }
                        MenuItem::action(e.name.clone(), SwitchKeymap { name: Some(e.name) })
                    })
                    .collect();
                (items, selected)
            }
        }
    }

    /// Opens the list of `combo` above it (the one open closes).
    pub fn open_list(&mut self, combo: Combo, window: &mut Window, cx: &mut Context<Self>) {
        self.close_list(window, cx);
        window.focus(&self.focus);
        let (items, selected) = Self::rows(combo, cx);
        let options = PopupOptions {
            selected,
            min_width: Some(LIST_MIN_WIDTH),
            row_height: Some(LIST_ROW),
            inactive_selection: true,
            ..PopupOptions::default()
        };
        let menu = ContextMenu::popup(items, options, window, cx);
        let subscription = cx.subscribe(&menu, |this, _, _: &DismissEvent, cx| {
            this.list = None;
            cx.notify();
        });
        self.list = Some(OpenList {
            combo,
            menu,
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(list) = self.list.take() {
            list.menu.update(cx, |m, cx| m.dismiss(window, cx));
            cx.notify();
        }
    }

    fn toggle_list(&mut self, combo: Combo, window: &mut Window, cx: &mut Context<Self>) {
        if self.list.as_ref().is_some_and(|l| l.combo == combo) {
            self.close_list(window, cx);
        } else {
            self.open_list(combo, window, cx);
        }
    }

    /// What the tab shows, for `workbench.state`.
    pub fn state_json(&self, cx: &App) -> Json {
        json!({
            "kind": "welcome",
            "title": self.title.as_ref(),
            "subtitle": SUBTITLE,
            "tiles": self.tiles.iter().map(|t| json!({
                "label": t.label,
                "command": t.command,
                "enabled": t.command.is_some(),
            })).collect::<Vec<_>>(),
            "focused_tile": self.focused_tile,
            "theme": format!("{THEME_PREFIX}{}", Self::theme_name(cx)),
            "keymap": format!("{KEYMAP_PREFIX}{}", keymaps::active_name(cx)),
            "list": self.list.as_ref().map(|l| json!({
                "combo": l.combo.name(),
                "menu": l.menu.read(cx).state_json(),
            })),
        })
    }

    // ---- drawing ----------------------------------------------------------------

    /// White (dark themes) or black at `dark` or `light` opacity.
    fn wash(dark: f32, light: f32, cx: &App) -> Hsla {
        if cx.theme().is_dark() {
            gpui::white().opacity(dark)
        } else {
            gpui::black().opacity(light)
        }
    }

    fn render_tile(&self, ix: usize, tile: WelcomeTile, window: &Window, cx: &App) -> AnyElement {
        let theme = cx.theme();
        // Focused: the pressed background and the focus ring
        // (`backgroundFocused`, `focusOutline`).
        let focus = &self.tile_focus[ix];
        let focused = focus.is_focused(window);
        let icon = Icon::new(tile.icon).size(px(TILE_ICON));
        let icon = match tile.tint {
            TileTint::Accent => icon.tint(theme.color(UiRole::TextLink)),
            TileTint::Own => icon,
        };
        let enabled = tile.command.is_some();
        div()
            .id(("welcome-tile", ix))
            .debug_selector(move || format!("welcome-tile-{ix}"))
            .flex_none()
            .w(px(TILE_WIDTH))
            .h(px(TILE_HEIGHT))
            .px(px(16.0))
            .py(px(12.0))
            .rounded(px(TILE_RADIUS))
            .track_focus(focus)
            .border(px(FOCUS_WIDTH))
            .border_color(if focused {
                theme.color(UiRole::Focus)
            } else {
                gpui::transparent_black()
            })
            .bg(if focused {
                Self::wash(0.18, 0.12, cx)
            } else {
                Self::wash(0.07, 0.05, cx)
            })
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(
                div()
                    .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
                    .child(icon.into_any_element()),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .text_size(px(SUBTITLE_SIZE))
                    .line_height(px(SUBTITLE_LINE))
                    .text_center()
                    .when(!enabled, |d| {
                        d.text_color(theme.color(UiRole::TextDisabled))
                    })
                    .child(tile.label),
            )
            .when_some(tile.command, |d, command| {
                d.when(!focused, |d| d.hover(|s| s.bg(Self::wash(0.12, 0.08, cx))))
                    .active(|s| s.bg(Self::wash(0.18, 0.12, cx)))
                    .on_click(move |_: &ClickEvent, window, cx| {
                        let args = tile
                            .args
                            .and_then(|a| serde_json::from_str(a).ok())
                            .unwrap_or(Json::Null);
                        if let Err(e) = ui::command::invoke(command, args, window, cx) {
                            eprintln!("welcome: {command}: {e:?}");
                        }
                    })
            })
            .when(!enabled, |d| d.tooltip(Tooltip::text(NOT_AVAILABLE, None)))
            .into_any_element()
    }

    fn render_combo(&self, combo: Combo, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (icon, label) = match combo {
            Combo::Theme => (
                if theme.is_dark() {
                    IconName::DARK_THEME
                } else {
                    IconName::LIGHT_THEME
                },
                format!("{THEME_PREFIX}{}", Self::theme_name(cx)),
            ),
            Combo::Keymap => (
                IconName::KEYBOARD,
                format!("{KEYMAP_PREFIX}{}", keymaps::active_name(cx)),
            ),
        };
        // The open list, its bottom left corner 4 above the box's top left.
        let list = self.list.as_ref().filter(|l| l.combo == combo).map(|l| {
            div().absolute().top_0().left_0().size_0().child(
                deferred(
                    anchored()
                        .anchor(Corner::BottomLeft)
                        .offset(point(px(0.0), px(-4.0)))
                        .snap_to_window_with_margin(px(4.0))
                        .child(l.menu.clone()),
                )
                .with_priority(2),
            )
        });
        let open = list.is_some();
        div()
            .id(("welcome-combo", combo.ix()))
            .debug_selector(move || format!("welcome-{}", combo.name()))
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .h(px(COMBO_HEIGHT))
            .pl(px(6.0))
            .pr(px(4.0))
            .gap(px(6.0))
            .rounded(px(4.0))
            .when(open, |d| d.bg(Self::wash(0.18, 0.12, cx)))
            .when(!open, |d| d.hover(|s| s.bg(Self::wash(0.12, 0.08, cx))))
            .child(Icon::new(icon))
            .child(div().whitespace_nowrap().child(label))
            .child(div().ml(px(2.0)).child(Icon::new(IconName::CHEVRON_DOWN)))
            .children(list)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_list(combo, window, cx)
            }))
            .into_any_element()
    }

    /// Our own glow at the top (the reference shows a blurred picture of
    /// its own there): two soft blobs above the top edge, teal and green.
    fn glow(cx: &App) -> gpui::Div {
        let dark = cx.theme().is_dark();
        let alpha = if dark { 0.8 } else { 0.3 };
        let blob = |x: f32, hue: f32| {
            div()
                .absolute()
                .top(px(-GLOW_SIZE * 0.55))
                .left(gpui::relative(x))
                .ml(px(-GLOW_SIZE / 2.0))
                .size(px(GLOW_SIZE))
                .rounded_full()
                .shadow(vec![BoxShadow {
                    color: gpui::hsla(hue, 0.5, 0.4, alpha),
                    offset: point(px(0.0), px(0.0)),
                    blur_radius: px(GLOW_SIZE * 0.5),
                    spread_radius: px(0.0),
                }])
        };
        div()
            .absolute()
            .inset_0()
            .child(blob(0.35, 0.47))
            .child(blob(0.65, 0.27))
    }
}

impl TabView for WelcomeView {
    fn tab_title(&self, _: &App) -> SharedString {
        self.title.clone()
    }

    fn tab_icon(&self, _: &App) -> IconName {
        self.icon
    }

    fn tab_state(&self, cx: &App) -> Json {
        self.state_json(cx)
    }

    fn title_in_toolbar(&self, _: &App) -> bool {
        false
    }
}

impl Focusable for WelcomeView {
    /// The first tile's: selecting the tab focuses it.
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.tile_focus.first().unwrap_or(&self.focus).clone()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let background = theme.color(UiRole::EditorBackground);
        let text = theme.color(UiRole::Text);
        let subtitle = theme.color(UiRole::TextDisabled);
        self.focused_tile = self.tile_focus.iter().position(|f| f.is_focused(window));
        let rows: Vec<AnyElement> =
            self.tiles
                .chunks(TILES_PER_ROW)
                .enumerate()
                .map(|(row, tiles)| {
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(TILE_GAP))
                        .children(tiles.iter().enumerate().map(|(i, t)| {
                            self.render_tile(row * TILES_PER_ROW + i, *t, window, cx)
                        }))
                        .into_any_element()
                })
                .collect();
        let combos = [
            self.render_combo(Combo::Theme, cx),
            self.render_combo(Combo::Keymap, cx),
        ];
        // A click outside an open list closes it (and nothing else).
        let backdrop = self.list.as_ref().map(|l| {
            let menu = l.menu.clone();
            let size = window.viewport_size();
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("welcome-backdrop")
                        .occlude()
                        .w(size.width)
                        .h(size.height)
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            menu.update(cx, |m, cx| m.dismiss(window, cx))
                        }),
                ),
            )
            .with_priority(1)
        });
        div()
            .id("welcome")
            .debug_selector(|| "welcome".into())
            .key_context("Welcome")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .text_color(text)
            .child(Self::glow(cx))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex_1()
                            .min_h(px(0.0))
                            .p(px(16.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .debug_selector(|| "welcome-title".into())
                                    .text_size(px(TITLE_SIZE))
                                    .line_height(px(TITLE_LINE))
                                    .child(self.title.clone()),
                            )
                            .child(
                                div()
                                    .mt(px(8.0))
                                    .text_size(px(SUBTITLE_SIZE))
                                    .line_height(px(SUBTITLE_LINE))
                                    .text_color(subtitle)
                                    .child(SUBTITLE),
                            )
                            .child(
                                div()
                                    .mt(px(32.0))
                                    .flex()
                                    .flex_col()
                                    .gap(px(TILE_GAP))
                                    .children(rows),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pb(px(FOOTER_BOTTOM))
                            .flex()
                            .flex_row()
                            .justify_center()
                            .gap(px(COMBO_GAP))
                            .children(combos),
                    ),
            )
            .children(backdrop)
    }
}
