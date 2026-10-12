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
    App, Global, ImageSource, IntoElement, Pixels, RenderImage, RenderOnce, Styled, Window, div,
    img, px,
};
use resvg::{tiny_skia, usvg};

use crate::theme::ThemeExt;

/// Name of a registered icon: its path below `assets/icons/` without the
/// `.svg` suffix, for example `general/chevronDown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IconName(pub &'static str);

impl IconName {
    pub const ADD: Self = Self("general/add");
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
    pub const BRANCH: Self = Self("vcs/branch");
    pub const FOLDER: Self = Self("nodes/folder");
    pub const MODULE: Self = Self("nodes/module");
    pub const EXCLUDED_FOLDER: Self = Self("nodes/excludeRoot");
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
    pub const IN_SELECTION: Self = Self("actions/inSelection");
    pub const MATCH_CASE: Self = Self("inline/matchCase");
    pub const WORDS: Self = Self("inline/exactWords");
    pub const REGEX: Self = Self("inline/regex");
    pub const PRESERVE_CASE: Self = Self("inline/preserveCase");
    pub const SEARCH_HISTORY: Self = Self("inline/searchHistory");
    pub const NEW_LINE: Self = Self("inline/newLine");
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
        "actions/inSelection",
        "fileTypes/archive", "fileTypes/binaryData", "fileTypes/config", "fileTypes/css",
        "fileTypes/csv", "fileTypes/docker", "fileTypes/html", "fileTypes/image",
        "fileTypes/javaScript", "fileTypes/json", "fileTypes/markdown", "fileTypes/rst",
        "fileTypes/shell", "fileTypes/sql", "fileTypes/text", "fileTypes/toml",
        "fileTypes/unknown", "fileTypes/xml", "fileTypes/yaml",
        "general/add", "general/chevronDown", "general/chevronLeft", "general/chevronRight",
        "general/chevronUp", "general/close", "general/closeSmall", "general/closeSmallHovered",
        "general/collapseAll", "general/down", "general/expandAll", "general/filter", "general/hide",
        "general/inspections/inspectionsOK", "general/locate", "general/locked",
        "general/moreHorizontal", "general/moreVertical", "general/pin", "general/search@20x20", "general/settings@20x20", "general/showAsTree", "general/up",
        "inline/exactWords", "inline/matchCase", "inline/newLine", "inline/preserveCase", "inline/regex",
        "inline/searchHistory",
        "nodes/excludeRoot", "nodes/folder", "nodes/module", "nodes/symlink",
        "run/debug", "run/run",
        "toolwindows/bookmarks@20x20", "toolwindows/commit@20x20", "toolwindows/debug@20x20",
        "toolwindows/find@20x20", "toolwindows/notifications@20x20",
        "toolwindows/problems@20x20", "toolwindows/project@20x20", "toolwindows/run@20x20",
        "toolwindows/services@20x20", "toolwindows/structure@20x20",
        "toolwindows/terminal@20x20", "toolwindows/todo@20x20", "toolwindows/vcs@20x20",
        "vcs/branch",
        "windows/close", "windows/maximize", "windows/minimize", "windows/restore",
    ],
    light_only: ["fileTypes/gitignore"]
}

/// A rasterized icon: name, dark variant, size in device pixels.
type ImageKey = (&'static str, bool, u32);

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
/// the window's scale, rasterized once and cached.
fn image(name: IconName, size: Pixels, window: &Window, cx: &App) -> Option<Arc<RenderImage>> {
    let icons = cx.try_global::<Icons>()?;
    let dark = cx.theme().is_dark();
    let pixels = (f32::from(size) * window.scale_factor()).round().max(1.0) as u32;
    let key = (name.0, dark, pixels);
    if let Some(image) = icons.images.borrow().get(&key) {
        return image.clone();
    }
    let source = icons.sources.get(name.0)?;
    let svg = if dark {
        source.dark.unwrap_or(source.light)
    } else {
        source.light
    };
    let image = rasterize(svg, pixels).map(Arc::new);
    icons.images.borrow_mut().insert(key, image.clone());
    image
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
}

impl Icon {
    /// A 16 px icon.
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: px(16.0),
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let size = self.size;
        match image(self.name, size, window, cx) {
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
