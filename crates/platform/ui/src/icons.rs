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
    pub const SETTINGS: Self = Self("general/settings@20x20");
    pub const RUN: Self = Self("run/run");
    pub const DEBUG: Self = Self("run/debug");
    pub const RERUN: Self = Self("run/rerun");
    pub const STOP: Self = Self("run/stop");
    pub const KILL_PROCESS: Self = Self("run/killProcess");
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
    pub const BRANCH: Self = Self("vcs/branch");
    pub const FOLDER: Self = Self("nodes/folder");
    pub const PACKAGE: Self = Self("nodes/package");
    pub const PREVIEW: Self = Self("general/previewHorizontally");
    pub const OPEN_IN_TOOL_WINDOW: Self = Self("general/openInToolWindow");
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
        "actions/checked", "actions/inSelection", "actions/newFolder", "actions/stopAndRestart",
        "codeInsight/intentionBulb", "codeInsight/quickfixBulb",
        "fileTypes/archive", "fileTypes/binaryData", "fileTypes/config", "fileTypes/css",
        "fileTypes/csv", "fileTypes/docker", "fileTypes/html", "fileTypes/image",
        "fileTypes/javaScript", "fileTypes/json", "fileTypes/markdown", "fileTypes/rst",
        "fileTypes/shell", "fileTypes/sql", "fileTypes/text", "fileTypes/toml",
        "fileTypes/unknown", "fileTypes/xml", "fileTypes/yaml",
        "general/add", "general/chevronDown", "general/chevronLeft", "general/chevronRight",
        "general/chevronUp", "general/close", "general/closeSmall", "general/closeSmallHovered",
        "general/collapseAll", "general/copy", "general/cut", "general/delete", "general/down", "general/expandAll", "general/filter",
        "general/help", "general/hide",
        "general/inspections/inspectionsOK", "general/locate", "general/locked",
        "general/moreHorizontal", "general/moreVertical", "general/openInToolWindow", "general/paste", "general/pin",
        "general/previewHorizontally", "general/print", "general/questionDialog", "general/refresh", "general/remove",
        "general/save", "general/scrollDown",
        "general/search@20x20", "general/warningDialog",
        "general/settings", "general/settings@20x20", "general/softWrap",
        "general/showAsTree", "general/up",
        "inline/browse", "inline/exactWords", "inline/expand", "inline/inlineSettings",
        "inline/matchCase", "inline/newLine", "inline/preserveCase", "inline/regex",
        "inline/searchHistory", "inline/variables",
        "json/object",
        "nodes/class", "nodes/constant", "nodes/enum", "nodes/errorMark", "nodes/excludeRoot",
        "nodes/field",
        "nodes/folder", "nodes/function", "nodes/interface", "nodes/method", "nodes/module",
        "nodes/package", "nodes/parameter", "nodes/property", "nodes/symlink", "nodes/template",
        "nodes/type", "nodes/variable", "nodes/word",
        "nodes/sourceRoot", "nodes/testRoot",
        "objectBrowser/sortAlphabetically",
        "run/debug", "run/killProcess", "run/rerun", "run/run", "run/stop",
        "status/error",
        "toolwindows/bookmarks@20x20", "toolwindows/commit@20x20", "toolwindows/debug@20x20",
        "toolwindows/find@20x20", "toolwindows/notifications@20x20",
        "toolwindows/problems@20x20", "toolwindows/project@20x20", "toolwindows/run@20x20",
        "toolwindows/services@20x20", "toolwindows/structure@20x20",
        "toolwindows/terminal@20x20", "toolwindows/todo@20x20", "toolwindows/vcs@20x20",
        "vcs/branch",
        "windows/close", "windows/maximize", "windows/minimize", "windows/restore",
    ],
    light_only: [
        "fileTypes/gitignore", "run/debug_stroke", "run/rerun_stroke", "run/run_stroke",
        "run/stop_stroke",
        "general/inspections/inspectionsError", "general/inspections/inspectionsWarning",
    ]
}

/// A rasterized icon: name, dark variant, size in device pixels, tint
/// (RGBA).
type ImageKey = (&'static str, bool, u32, Option<u32>);

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
    let icons = cx.try_global::<Icons>()?;
    let dark = cx.theme().is_dark();
    let pixels = (f32::from(size) * window.scale_factor()).round().max(1.0) as u32;
    let tint = tint.map(rgba8);
    let key = (name.0, dark, pixels, tint.map(u32::from_be_bytes));
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
        None => rasterize(svg, pixels),
    }
    .map(Arc::new);
    icons.images.borrow_mut().insert(key, image.clone());
    image
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
