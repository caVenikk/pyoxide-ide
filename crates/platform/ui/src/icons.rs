//! Colour SVG icons, rasterized with `resvg` at the window's device scale
//! (GPUI's own `img()` rasterizes SVG at 1x, and `svg()` is monochrome).
//!
//! The platform icons are the New UI icons of the IntelliJ Platform
//! Community repository (Apache-2.0, see NOTICE), with a light and a dark
//! variant each. Products register their own icons (file types, tool
//! windows) with [`register`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, Global, Hsla, ImageSource, IntoElement, Pixels, RenderImage, RenderOnce, Styled, Window,
    div, img, px,
};
use resvg::{tiny_skia, usvg};

use crate::theme::ThemeExt;

/// Name of a registered icon: its path below `assets/icons/` without the
/// `.svg` suffix, for example `general/chevronDown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IconName(pub &'static str);

impl IconName {
    pub const ADD: Self = Self("general/add");
    pub const CUT: Self = Self("general/cut");
    pub const PASTE: Self = Self("general/paste");
    pub const REFRESH: Self = Self("general/refresh");
    /// Split Right and Split Down.
    pub const SPLIT_VERTICALLY: Self = Self("general/splitVertically");
    pub const SPLIT_HORIZONTALLY: Self = Self("general/splitHorizontally");
    pub const CHEVRON_DOWN: Self = Self("general/chevronDown");
    pub const CHEVRON_LEFT: Self = Self("general/chevronLeft");
    pub const CHEVRON_RIGHT: Self = Self("general/chevronRight");
    pub const CHEVRON_UP: Self = Self("general/chevronUp");
    pub const CLOSE: Self = Self("general/close");
    pub const CLOSE_SMALL: Self = Self("general/closeSmall");
    pub const CLOSE_SMALL_HOVERED: Self = Self("general/closeSmallHovered");
    pub const COLLAPSE_ALL: Self = Self("general/collapseAll");
    pub const EXPAND_ALL: Self = Self("general/expandAll");
    pub const HIDE: Self = Self("general/hide");
    pub const INSPECTIONS_OK: Self = Self("general/inspections/inspectionsOK");
    pub const LOCATE: Self = Self("general/locate");
    pub const LOCKED: Self = Self("general/locked");
    pub const MORE_VERTICAL: Self = Self("general/moreVertical");
    pub const PIN: Self = Self("general/pin");
    pub const MORE_HORIZONTAL: Self = Self("general/moreHorizontal");
    pub const SHOW_AS_TREE: Self = Self("general/showAsTree");
    pub const SEARCH: Self = Self("general/search@20x20");
    /// The 16 by 16 search icon (`AllIcons.Actions.Search`: the Welcome
    /// tab's search field).
    pub const SEARCH_SMALL: Self = Self("general/search");
    pub const SETTINGS: Self = Self("general/settings@20x20");
    pub const RUN: Self = Self("run/run");
    pub const DEBUG: Self = Self("run/debug");
    pub const RERUN: Self = Self("run/rerun");
    pub const STOP: Self = Self("run/stop");
    pub const KILL_PROCESS: Self = Self("run/killProcess");
    /// The cancel button of a background task and under the pointer (the
    /// New UI's `AllIcons.Process.Stop`, `StopHovered`).
    pub const PROGRESS_STOP: Self = Self("progress/stop");
    pub const PROGRESS_STOP_HOVERED: Self = Self("progress/stopHovered");
    /// The pause button of a pausable background task, and its resume
    /// button while paused (`AllIcons.Process.ProgressPause`, `ProgressResume`).
    pub const PROGRESS_PAUSE: Self = Self("progress/pause");
    pub const PROGRESS_PAUSE_HOVERED: Self = Self("progress/pauseHovered");
    pub const PROGRESS_RESUME: Self = Self("progress/resume");
    pub const PROGRESS_RESUME_HOVERED: Self = Self("progress/resumeHovered");
    /// White variants for the coloured buttons of the run widget.
    pub const RUN_STROKE: Self = Self("run/run_stroke");
    pub const RERUN_STROKE: Self = Self("run/rerun_stroke");
    pub const STOP_STROKE: Self = Self("run/stop_stroke");
    /// Outlines for disabled run widget buttons, tinted grey (the
    /// reference paints its run widget icons as strokes, `toStrokeIcon`,
    /// and dims them when disabled).
    pub const DEBUG_STROKE: Self = Self("run/debug_stroke");
    pub const DELETE: Self = Self("general/delete");
    pub const PRINT: Self = Self("general/print");
    pub const SOFT_WRAP: Self = Self("general/softWrap");
    pub const SCROLL_DOWN: Self = Self("general/scrollDown");
    pub const QUESTION_DIALOG: Self = Self("general/questionDialog");
    pub const WARNING_DIALOG: Self = Self("general/warningDialog");
    pub const ERROR_DIALOG: Self = Self("general/errorDialog");
    /// The cross of a recent project whose folder is gone, and under the
    /// pointer (`AllIcons.Welcome.RecentProjects.Remove`, `RemoveHover`:
    /// 10 by 10, no New UI icon of their own).
    pub const RECENT_PROJECT_REMOVE: Self = Self("welcome/recentProjects/remove");
    pub const RECENT_PROJECT_REMOVE_HOVER: Self = Self("welcome/recentProjects/remove-hover");
    pub const BRANCH: Self = Self("vcs/branch");
    /// Rollback and Show Diff of the popup of a change marker (the New
    /// UI's `AllIcons.Actions.Rollback` and `Diff`).
    pub const ROLLBACK: Self = Self("vcs/revert");
    pub const DIFF: Self = Self("vcs/diff");
    /// The Commit tool window's toolbar: Shelve Silently, View Options and
    /// the commit message history (the New UI's `Vcs.ShelveSilent`,
    /// `Actions.Show`, `Vcs.History`).
    pub const SHELVE: Self = Self("vcs/shelve");
    pub const SHOW: Self = Self("general/show");
    pub const HISTORY: Self = Self("general/history");
    /// The changelists group of the marker popup (`AllIcons.Vcs.Changelist`)
    /// with the corner arrow of a popup group (`AllIcons.General.Dropdown`),
    /// and the buttons of its commit field (`PlatformVcsImplIcons.AmendInline`,
    /// `CommitInline`).
    pub const CHANGELIST: Self = Self("vcs/changelist");
    pub const DROPDOWN_CORNER: Self = Self("general/dropdown");
    pub const AMEND_INLINE: Self = Self("vcs/amendInline");
    pub const COMMIT_INLINE: Self = Self("vcs/commitInline");
    /// The VCS widget and the branches of its popup (the New UI's
    /// `AllIcons.General.Vcs`, `Vcs.BranchNode`), a tag
    /// (`DvcsImplIcons.BranchLabel`), the current branch with and without
    /// the favourite star, a favourite (`AllIcons.Nodes.Favorite`).
    pub const VCS: Self = Self("general/vcs");
    pub const BRANCH_LABEL: Self = Self("vcs/branchLabel");
    pub const CURRENT_BRANCH: Self = Self("vcs/currentBranchLabel");
    pub const CURRENT_BRANCH_FAVORITE: Self = Self("vcs/currentBranchFavoriteLabel");
    pub const FAVORITE: Self = Self("nodes/star");
    /// Incoming and outgoing commits (`DvcsImplIcons.Incoming`,
    /// `Outgoing`, 12 px).
    pub const INCOMING: Self = Self("vcs/changesUpdate@12x12");
    pub const OUTGOING: Self = Self("vcs/changesPush@12x12");
    /// Fetch, Update Project, Commit and Push (`AllIcons.Vcs.Fetch`,
    /// `Actions.CheckOut`, `Actions.Commit`, `Vcs.Push`).
    pub const FETCH: Self = Self("vcs/fetch");
    pub const VCS_UPDATE: Self = Self("vcs/update");
    pub const COMMIT: Self = Self("vcs/commit");
    pub const PUSH: Self = Self("vcs/push");
    pub const BREAKPOINT: Self = Self("breakpoints/breakpoint");
    pub const FOLDER: Self = Self("nodes/folder");
    pub const PACKAGE: Self = Self("nodes/package");
    pub const PREVIEW: Self = Self("general/previewHorizontally");
    pub const OPEN_IN_TOOL_WINDOW: Self = Self("general/openInToolWindow");
    /// Where a side tool window in split mode sits, at the right
    /// (`AllIcons.Actions.MoveToRightBottom`): Open in Documentation Tool
    /// Window of the documentation popup.
    pub const MOVE_TO_RIGHT_BOTTOM: Self = Self("actions/moveToRightBottom");
    pub const MODULE: Self = Self("nodes/module");
    pub const EXCLUDED_FOLDER: Self = Self("nodes/excludeRoot");
    /// A sources root and a test sources root (`AllIcons.Modules.SourceRoot`,
    /// `TestRoot`).
    pub const SOURCE_ROOT: Self = Self("nodes/sourceRoot");
    pub const TEST_ROOT: Self = Self("nodes/testRoot");
    pub const SYMLINK: Self = Self("nodes/symlink");
    pub const TOOL_PROJECT: Self = Self("toolwindows/project@20x20");
    pub const TOOL_COMMIT: Self = Self("toolwindows/commit@20x20");
    pub const TOOL_STRUCTURE: Self = Self("toolwindows/structure@20x20");
    pub const TOOL_BOOKMARKS: Self = Self("toolwindows/bookmarks@20x20");
    pub const TOOL_PROBLEMS: Self = Self("toolwindows/problems@20x20");
    pub const TOOL_SERVICES: Self = Self("toolwindows/services@20x20");
    pub const TOOL_VCS: Self = Self("toolwindows/vcs@20x20");
    pub const TOOL_NOTIFICATIONS: Self = Self("toolwindows/notifications@20x20");
    pub const TOOL_RUN: Self = Self("toolwindows/run@20x20");
    pub const TOOL_DEBUG: Self = Self("toolwindows/debug@20x20");
    pub const TOOL_FIND: Self = Self("toolwindows/find@20x20");
    pub const TOOL_TODO: Self = Self("toolwindows/todo@20x20");
    pub const TOOL_TERMINAL: Self = Self("toolwindows/terminal@20x20");
    /// The Documentation tool window (`AllIcons.Toolwindows.Documentation`).
    pub const TOOL_DOCUMENTATION: Self = Self("toolwindows/documentation@20x20");
    pub const FILE_TEXT: Self = Self("fileTypes/text");
    pub const FILE_UNKNOWN: Self = Self("fileTypes/unknown");
    pub const FILE_BINARY: Self = Self("fileTypes/binaryData");
    pub const WINDOW_MINIMIZE: Self = Self("windows/minimize");
    pub const WINDOW_MAXIMIZE: Self = Self("windows/maximize");
    pub const WINDOW_RESTORE: Self = Self("windows/restore");
    pub const WINDOW_CLOSE: Self = Self("windows/close");
    pub const ARROW_UP: Self = Self("general/up");
    pub const ARROW_DOWN: Self = Self("general/down");
    pub const FILTER: Self = Self("general/filter");
    pub const CHECKED: Self = Self("actions/checked");
    pub const IN_SELECTION: Self = Self("actions/inSelection");
    pub const SHORTCUT_FILTER: Self = Self("actions/shortcutFilter");
    pub const MATCH_CASE: Self = Self("inline/matchCase");
    pub const WORDS: Self = Self("inline/exactWords");
    pub const REGEX: Self = Self("inline/regex");
    pub const PRESERVE_CASE: Self = Self("inline/preserveCase");
    pub const SEARCH_HISTORY: Self = Self("inline/searchHistory");
    pub const NEW_LINE: Self = Self("inline/newLine");
    // Code intelligence: symbol kinds of completion items (the reference's
    // `LspCompletionCustomizer.getIcon`; keywords, modules and values have
    // none), the intention bulbs and the Language Services widget's `{}`.
    pub const NODE_CLASS: Self = Self("nodes/class");
    pub const NODE_FUNCTION: Self = Self("nodes/function");
    pub const NODE_METHOD: Self = Self("nodes/method");
    pub const NODE_FIELD: Self = Self("nodes/field");
    pub const NODE_VARIABLE: Self = Self("nodes/variable");
    pub const NODE_PARAMETER: Self = Self("nodes/parameter");
    pub const NODE_PROPERTY: Self = Self("nodes/property");
    pub const NODE_INTERFACE: Self = Self("nodes/interface");
    pub const NODE_ENUM: Self = Self("nodes/enum");
    pub const NODE_CONSTANT: Self = Self("nodes/constant");
    pub const NODE_TYPE: Self = Self("nodes/type");
    pub const NODE_TEMPLATE: Self = Self("nodes/template");
    pub const NODE_WORD: Self = Self("nodes/word");
    /// A line found by text search (`AllIcons.Nodes.TextArea`).
    pub const TEXT_AREA: Self = Self("nodes/textArea");
    pub const INTENTION_BULB: Self = Self("codeInsight/intentionBulb");
    pub const QUICKFIX_BULB: Self = Self("codeInsight/quickfixBulb");
    pub const JSON_OBJECT: Self = Self("json/object");
    /// Severity marks: white shapes the reference colours with the
    /// severity's error stripe colour (`HighlightDisplayLevel`); a weak
    /// warning uses the warning shape.
    pub const INSPECTIONS_ERROR: Self = Self("general/inspections/inspectionsError");
    pub const INSPECTIONS_WARNING: Self = Self("general/inspections/inspectionsWarning");
    // The Language Services widget and popup: stop and restart a server,
    // its settings, the error output of a terminated one, and the error
    // mark on the icon of a terminated server.
    pub const STOP_AND_RESTART: Self = Self("actions/stopAndRestart");
    pub const SETTINGS_SMALL: Self = Self("general/settings");
    pub const ERROR: Self = Self("status/error");
    /// Balloon icons of information and warning notifications.
    pub const INFO: Self = Self("status/info");
    pub const WARNING: Self = Self("status/warning");
    /// `AllIcons.Status.Success` (all background tasks completed).
    pub const STATUS_SUCCESS: Self = Self("status/success");
    pub const ERROR_MARK: Self = Self("nodes/errorMark");
    // The toolbar of the Run/Debug Configurations dialog.
    pub const REMOVE: Self = Self("general/remove");
    pub const COPY: Self = Self("general/copy");
    pub const SAVE: Self = Self("general/save");
    pub const NEW_FOLDER: Self = Self("actions/newFolder");
    pub const SORT: Self = Self("objectBrowser/sortAlphabetically");
    // Its form: field buttons (Insert Macros, Browse, Expand), the gear of
    // "Store as project file" and the help button.
    pub const MACROS: Self = Self("inline/variables");
    pub const BROWSE: Self = Self("inline/browse");
    pub const EXPAND: Self = Self("inline/expand");
    pub const INLINE_SETTINGS: Self = Self("inline/inlineSettings");
    pub const HELP: Self = Self("general/help");
    /// The circled "?" after an option with a help tooltip
    /// (`AllIcons.General.ContextHelp`, `ContextHelpLabel`).
    pub const CONTEXT_HELP: Self = Self("general/questionMark");
    // The Settings window: Back and Forward of its banner, the mark of
    // pages with project settings.
    pub const BACK: Self = Self("general/left");
    pub const FORWARD: Self = Self("general/right");
    pub const PROJECT_CONFIGURABLE: Self = Self("general/projectConfigurable");
    /// The arrow after a link to a web page (`AllIcons.Ide.External_link_arrow`).
    pub const EXTERNAL_LINK: Self = Self("ide/externalLink");
    /// Jump to Source in a diff (`AllIcons.Actions.EditSource`).
    pub const EDIT: Self = Self("general/edit");
    /// The diff viewer's modes.
    pub const DIFF_SIDE_BY_SIDE: Self = Self("diff/sideBySide");
    pub const DIFF_UNIFIED: Self = Self("diff/unified");
    /// The Welcome tab: Open File, and the Theme (dark or light) and
    /// Keymap combo boxes (`AllIcons.General.Upload`,
    /// `MeetNewUi.DarkTheme`, `MeetNewUi.LightTheme`, `General.Keyboard`).
    pub const UPLOAD: Self = Self("general/upload");
    pub const DARK_THEME: Self = Self("meetNewUi/darkTheme");
    pub const LIGHT_THEME: Self = Self("meetNewUi/lightTheme");
    pub const KEYBOARD: Self = Self("general/keyboard");
    /// The Welcome tab's Learn and Plugins tiles (`AllIcons.General.Learn`,
    /// `AllIcons.Nodes.Plugin`) and New Notebook's
    /// (`AllIcons.FileTypes.JupyterNotebook`).
    pub const LEARN: Self = Self("general/learn");
    pub const PLUGIN: Self = Self("nodes/plugin");
    pub const JUPYTER_NOTEBOOK: Self = Self("fileTypes/jupyterNotebook");
    /// Config or Installation Directory of the import dialog's Other
    /// Options (`StartupImportIcons.Icons.ConfigFile`, 20 by 20).
    pub const CONFIG_FILE: Self = Self("startup/configFile");
    /// A diff change's check box in the gutter and the title's
    /// (`AllIcons.Diff.GutterCheckBox*`).
    pub const DIFF_CHECKBOX: Self = Self("diff/gutterCheckBox@14x14");
    pub const DIFF_CHECKBOX_SELECTED: Self = Self("diff/gutterCheckBoxSelected@14x14");
    pub const DIFF_CHECKBOX_INDETERMINATE: Self = Self("diff/gutterCheckBoxIndeterminate@14x14");
    /// Revert a diff change (`AllIcons.Diff.ArrowRight`, `>>`).
    pub const DIFF_ARROW_RIGHT: Self = Self("diff/arrowRight@14x14");
}

/// Source of one icon: the light variant and, when it differs, the dark one.
#[derive(Debug, Clone, Copy)]
pub struct IconSvg {
    pub name: &'static str,
    pub light: &'static [u8],
    pub dark: Option<&'static [u8]>,
}

macro_rules! platform_icons {
    (both: [$($both:literal),* $(,)?], light_only: [$($light:literal),* $(,)?]) => {
        const PLATFORM: &[IconSvg] = &[
            $(IconSvg {
                name: $both,
                light: include_bytes!(concat!("../assets/icons/", $both, ".svg")),
                dark: Some(include_bytes!(concat!("../assets/icons/", $both, "_dark.svg"))),
            },)*
            $(IconSvg {
                name: $light,
                light: include_bytes!(concat!("../assets/icons/", $light, ".svg")),
                dark: None,
            },)*
        ];
    };
}

platform_icons! {
    both: [
        "actions/checked", "actions/inSelection", "actions/moveToRightBottom", "actions/newFolder", "actions/shortcutFilter",
        "actions/stopAndRestart",
        "codeInsight/intentionBulb", "codeInsight/quickfixBulb",
        "diff/arrowRight@14x14", "diff/gutterCheckBox@14x14",
        "diff/gutterCheckBoxIndeterminate@14x14", "diff/gutterCheckBoxSelected@14x14",
        "diff/sideBySide", "diff/unified",
        "fileTypes/archive", "fileTypes/binaryData", "fileTypes/css", "fileTypes/csv",
        "fileTypes/docker", "fileTypes/dockerCompose", "fileTypes/dockerIgnore",
        "fileTypes/editorConfig", "fileTypes/html", "fileTypes/ignored", "fileTypes/image",
        "fileTypes/ini", "fileTypes/javaScript", "fileTypes/json", "fileTypes/jupyterNotebook",
        "fileTypes/markdown", "fileTypes/patch", "fileTypes/properties",
        "fileTypes/rst", "fileTypes/shell", "fileTypes/sql", "fileTypes/text", "fileTypes/toml",
        "fileTypes/unknown", "fileTypes/xml", "fileTypes/yaml",
        "general/add", "general/chevronDown", "general/chevronLeft", "general/chevronRight",
        "general/chevronUp", "general/close", "general/closeSmall", "general/closeSmallHovered",
        "general/collapseAll", "general/copy", "general/cut", "general/delete", "general/down",
        "general/dropdown", "general/edit", "general/expandAll", "general/filter",
        "general/help", "general/hide", "general/history", "general/splitHorizontally", "general/splitVertically",
        "general/inspections/inspectionsOK", "general/left", "general/locate", "general/locked",
        "general/moreHorizontal", "general/moreVertical", "general/openInToolWindow", "general/paste", "general/pin",
        "general/previewHorizontally", "general/print", "general/projectConfigurable",
        "general/questionDialog", "general/questionMark", "general/refresh", "general/remove", "general/right",
        "general/save", "general/scrollDown",
        "general/search", "general/search@20x20", "general/warningDialog", "general/errorDialog",
        "general/settings", "general/settings@20x20", "general/show", "general/softWrap",
        "general/showAsTree", "general/up", "general/upload", "general/vcs",
        "general/keyboard", "general/learn",
        "ide/externalLink",
        "inline/browse", "inline/exactWords", "inline/expand", "inline/inlineSettings",
        "inline/matchCase", "inline/newLine", "inline/preserveCase", "inline/regex",
        "inline/searchHistory", "inline/variables",
        "json/object",
        "meetNewUi/darkTheme", "meetNewUi/lightTheme",
        "nodes/class", "nodes/constant", "nodes/enum", "nodes/errorMark", "nodes/excludeRoot",
        "nodes/field",
        "nodes/folder", "nodes/function", "nodes/interface", "nodes/method", "nodes/module",
        "nodes/package", "nodes/parameter", "nodes/plugin", "nodes/property", "nodes/symlink", "nodes/template",
        "nodes/textArea", "nodes/type", "nodes/variable", "nodes/word",
        "nodes/sourceRoot", "nodes/star", "nodes/testRoot",
        "objectBrowser/sortAlphabetically",
        "progress/pause", "progress/pauseHovered", "progress/resume", "progress/resumeHovered",
        "progress/stop", "progress/stopHovered",
        "run/debug", "run/killProcess",
        "startup/configFile", "run/rerun", "run/run", "run/stop",
        "status/error", "status/info", "status/success", "status/warning",
        "toolwindows/bookmarks@20x20", "toolwindows/commit@20x20", "toolwindows/debug@20x20",
        "toolwindows/documentation@20x20",
        "toolwindows/find@20x20", "toolwindows/notifications@20x20",
        "toolwindows/problems@20x20", "toolwindows/project@20x20", "toolwindows/run@20x20",
        "toolwindows/services@20x20", "toolwindows/structure@20x20",
        "toolwindows/terminal@20x20", "toolwindows/todo@20x20", "toolwindows/vcs@20x20",
        "vcs/amendInline", "vcs/branch", "vcs/branchLabel", "vcs/changelist",
        "vcs/changesPush@12x12", "vcs/changesUpdate@12x12", "vcs/commitInline",
        "vcs/commit", "vcs/currentBranchFavoriteLabel", "vcs/currentBranchLabel", "vcs/diff",
        "vcs/fetch", "vcs/push", "vcs/revert", "vcs/shelve", "vcs/update", "vcs/vendors/github",
        "welcome/recentProjects/remove", "welcome/recentProjects/remove-hover",
        "windows/close", "windows/maximize", "windows/minimize", "windows/restore",
    ],
    light_only: [
        "breakpoints/breakpoint",
        "fileTypes/gitLabLogo", "run/debug_stroke", "run/rerun_stroke", "run/run_stroke",
        "run/stop_stroke",
        "general/inspections/inspectionsError", "general/inspections/inspectionsWarning",
    ]
}

/// A rasterized icon: name, dark variant, size in device pixels, tint
/// (RGBA), opacity (0-255), the theme's icon colours
/// ([`crate::theme::IconColors::key`]).
type ImageKey = (&'static str, bool, u32, Option<u32>, u8, u64);

/// Registered icons and the rasterized images.
#[derive(Default)]
struct Icons {
    sources: HashMap<&'static str, IconSvg>,
    images: RefCell<HashMap<ImageKey, Option<Arc<RenderImage>>>>,
}

impl Global for Icons {}

/// Registers the platform icons.
pub fn init(cx: &mut App) {
    if !cx.has_global::<Icons>() {
        cx.set_global(Icons::default());
    }
    register(cx, PLATFORM);
}

/// Adds or replaces icons, for example a product's file type icons.
pub fn register(cx: &mut App, icons: &[IconSvg]) {
    let registry = cx.default_global::<Icons>();
    for icon in icons {
        registry.sources.insert(icon.name, *icon);
    }
    registry.images.borrow_mut().clear();
}

/// Whether an icon of this name is registered.
pub fn exists(cx: &App, name: IconName) -> bool {
    cx.try_global::<Icons>()
        .is_some_and(|icons| icons.sources.contains_key(name.0))
}

/// Names of all platform icons (for tests and asset checks).
pub fn platform_names() -> impl Iterator<Item = &'static str> {
    PLATFORM.iter().map(|i| i.name)
}

/// The icon as an image of `size` logical pixels for the current theme and
/// the window's scale, rasterized once and cached; a `tint` colours every
/// pixel and keeps the shape's alpha.
fn image(
    name: IconName,
    size: Pixels,
    tint: Option<Hsla>,
    window: &Window,
    cx: &App,
) -> Option<Arc<RenderImage>> {
    image_with_opacity(name, size, tint, 1.0, window, cx)
}

/// The icon as an image for a custom element to paint
/// (`Window::paint_image`): `size` logical pixels square for the current
/// theme and the window's scale, optionally tinted, every pixel's alpha
/// times `opacity`; rasterized once and cached.
pub fn image_with_opacity(
    name: IconName,
    size: Pixels,
    tint: Option<Hsla>,
    opacity: f32,
    window: &Window,
    cx: &App,
) -> Option<Arc<RenderImage>> {
    let icons = cx.try_global::<Icons>()?;
    let theme = cx.theme();
    let dark = theme.is_dark();
    let pixels = (f32::from(size) * window.scale_factor()).round().max(1.0) as u32;
    let tint = tint.map(rgba8);
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    // A tint replaces every colour: the theme's icon colours do not matter.
    let colors = theme.icon_colors();
    let palette = if tint.is_some() { 0 } else { colors.key };
    let key = (
        name.0,
        dark,
        pixels,
        tint.map(u32::from_be_bytes),
        alpha,
        palette,
    );
    if let Some(image) = icons.images.borrow().get(&key) {
        return image.clone();
    }
    let source = icons.sources.get(name.0)?;
    let svg = if dark {
        source.dark.unwrap_or(source.light)
    } else {
        source.light
    };
    let image = match tint {
        Some(tint) => rasterize_tinted(svg, pixels, tint),
        None if palette != 0 => rasterize(&recolor(svg, &colors.pairs), pixels),
        None => rasterize(svg, pixels),
    }
    .map(|image| fade(image, alpha))
    .map(Arc::new);
    icons.images.borrow_mut().insert(key, image.clone());
    image
}

/// The image with every pixel's alpha times `alpha` / 255 (straight alpha).
fn fade(image: RenderImage, alpha: u8) -> RenderImage {
    if alpha == u8::MAX {
        return image;
    }
    let size = image.size(0);
    let Some(bytes) = image.as_bytes(0) else {
        return image;
    };
    let mut data = bytes.to_vec();
    for pixel in data.as_chunks_mut::<4>().0 {
        pixel[3] = ((u32::from(pixel[3]) * u32::from(alpha) + 127) / 255) as u8;
    }
    match image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, data) {
        Some(buffer) => RenderImage::new(smallvec::smallvec![image::Frame::new(buffer)]),
        None => image,
    }
}

/// The SVG with the theme's icon colours: every `#RRGGBB` (any case) that
/// is the first colour of a pair becomes the second (`#rrggbb`, or
/// `rgba(...)` when it is not opaque), as the reference patches the
/// `fill` and `stroke` of its icons (`UiThemePaletteScopeManager`).
pub fn recolor(svg: &[u8], pairs: &[([u8; 3], [u8; 4])]) -> Vec<u8> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(svg.len() + 16);
    let mut i = 0;
    while i < svg.len() {
        let rgb = (svg[i] == b'#' && i + 7 <= svg.len())
            .then(|| {
                let digits = &svg[i + 1..i + 7];
                let next_is_hex = svg.get(i + 7).copied().and_then(hex).is_some();
                let mut rgb = [0u8; 3];
                for (n, pair) in digits.chunks(2).enumerate() {
                    rgb[n] = hex(pair[0])? * 16 + hex(pair[1])?;
                }
                (!next_is_hex).then_some(rgb)
            })
            .flatten();
        match rgb.and_then(|rgb| pairs.iter().find(|(from, _)| *from == rgb)) {
            Some((_, [r, g, b, a])) => {
                let text = if *a == u8::MAX {
                    format!("#{r:02x}{g:02x}{b:02x}")
                } else {
                    format!("rgba({r},{g},{b},{:.3})", f32::from(*a) / 255.0)
                };
                out.extend_from_slice(text.as_bytes());
                i += 7;
            }
            None => {
                out.push(svg[i]);
                i += 1;
            }
        }
    }
    out
}

/// A colour as 8-bit RGBA.
fn rgba8(color: Hsla) -> [u8; 4] {
    let c = gpui::Rgba::from(color);
    [c.r, c.g, c.b, c.a].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Renders an SVG in one colour: the shape's alpha times the colour's (the
/// reference colours its white severity masks this way).
pub fn rasterize_tinted(svg: &[u8], pixels: u32, rgba: [u8; 4]) -> Option<RenderImage> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        pixels as f32 / size.width(),
        pixels as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    let [r, g, b, a] = rgba;
    for pixel in data.as_chunks_mut::<4>().0 {
        let alpha = (u32::from(pixel[3]) * u32::from(a) + 127) / 255;
        *pixel = [b, g, r, alpha as u8];
    }
    let buffer = image::RgbaImage::from_raw(pixels, pixels, data)?;
    Some(RenderImage::new(smallvec::smallvec![image::Frame::new(
        buffer
    )]))
}

/// Renders an SVG into a square image of `pixels` device pixels, as straight
/// alpha BGRA (the layout GPUI's sprite atlas expects).
pub fn rasterize(svg: &[u8], pixels: u32) -> Option<RenderImage> {
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(
        pixels as f32 / size.width(),
        pixels as f32 / size.height(),
    );
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut data = pixmap.take();
    for pixel in data.as_chunks_mut::<4>().0 {
        let a = u32::from(pixel[3]);
        if a > 0 && a < 255 {
            for c in &mut pixel[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(pixels, pixels, data)?;
    Some(RenderImage::new(smallvec::smallvec![image::Frame::new(
        buffer
    )]))
}

/// An icon element of a fixed square size.
#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: Pixels,
    tint: Option<Hsla>,
}

impl Icon {
    /// A 16 px icon.
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: px(16.0),
            tint: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    /// Draws the icon's shape in one colour (the white severity masks).
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = Some(color);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.size;
        match image(self.name, size, self.tint, window, cx) {
            Some(image) => img(ImageSource::Render(image))
                .flex_none()
                .size(size)
                .into_any_element(),
            None => div().flex_none().size(size).into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_icon_rasterizes() {
        for icon in PLATFORM {
            for svg in [Some(icon.light), icon.dark].into_iter().flatten() {
                let image = rasterize(svg, 32).unwrap_or_else(|| panic!("{}", icon.name));
                let bytes = image.as_bytes(0).expect("one frame");
                assert_eq!(bytes.len(), 32 * 32 * 4, "{}", icon.name);
                assert!(
                    bytes.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
                    "{} is empty",
                    icon.name
                );
            }
        }
    }

    #[test]
    fn a_tint_colours_the_shape_and_keeps_its_alpha() {
        let svg = br##"<svg width="2" height="2" viewBox="0 0 2 2" xmlns="http://www.w3.org/2000/svg"><rect width="1" height="2" fill="white"/></svg>"##;
        let image = rasterize_tinted(svg, 2, [0xd6, 0x4d, 0x5b, 0xff]).unwrap();
        let bytes = image.as_bytes(0).unwrap();
        // Blue, green, red, alpha: the left column is the colour, the right
        // one stays transparent.
        assert_eq!(&bytes[..4], &[0x5b, 0x4d, 0xd6, 0xff]);
        assert_eq!(bytes[7], 0);
        assert_eq!(rgba8(gpui::white()), [255, 255, 255, 255]);
    }

    #[test]
    fn the_theme_recolours_the_palette_colours_of_an_icon() {
        let svg = br##"<svg><path fill="#57965C" stroke="#253627"/><path fill="#57965c80"/><rect fill="#CED0D6"/></svg>"##;
        let pairs = [
            ([0x57, 0x96, 0x5C], [0xA6, 0xE2, 0x2E, 0xFF]),
            ([0x25, 0x36, 0x27], [0x27, 0x28, 0x22, 0x80]),
        ];
        let out = String::from_utf8(recolor(svg, &pairs)).unwrap();
        assert_eq!(
            out,
            r##"<svg><path fill="#a6e22e" stroke="rgba(39,40,34,0.502)"/><path fill="#57965c80"/><rect fill="#CED0D6"/></svg>"##
        );
        // The recoloured icon still renders.
        let svg = br##"<svg width="2" height="2" viewBox="0 0 2 2" xmlns="http://www.w3.org/2000/svg"><rect width="2" height="2" fill="#57965C"/></svg>"##;
        let image = rasterize(&recolor(svg, &pairs), 2).unwrap();
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0x2E, 0xE2, 0xA6, 0xFF]);
    }

    #[test]
    fn converts_to_straight_bgra() {
        let svg = br##"<svg width="2" height="2" viewBox="0 0 2 2" xmlns="http://www.w3.org/2000/svg"><rect width="2" height="2" fill="#ff8000" fill-opacity="0.5"/></svg>"##;
        let image = rasterize(svg, 2).unwrap();
        let px = &image.as_bytes(0).unwrap()[..4];
        // Blue, green, red, alpha; red stays near 255 after un-premultiplying.
        assert_eq!(px[0], 0);
        assert!((px[1] as i32 - 128).abs() <= 2, "{px:?}");
        assert!(px[2] >= 253, "{px:?}");
        assert!((px[3] as i32 - 128).abs() <= 1, "{px:?}");
    }
}
