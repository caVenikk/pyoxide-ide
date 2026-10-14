//! The component layer of Pyoxide IDE on GPUI (ADR 0006, 0007).
//!
//! Everything toolkit-specific that is shared by the editor and the
//! workbench lives here: the theme as GPUI colours and metrics, the bundled
//! fonts, colour icons rasterized at device scale, small components, context
//! menus and the UI command index. Keeping GPUI behind this layer keeps a later toolkit
//! upgrade contained.

pub mod command;
pub mod components;
pub mod fonts;
pub mod icons;
pub mod keys;
pub mod markdown;
pub mod menu;
pub mod pointer;
pub mod theme;

pub use icons::{Icon, IconName};
pub use theme::{Theme, ThemeExt};

/// Registers what every window needs: the bundled fonts, the platform icons
/// and the default theme (dark unless `light` is set).
pub fn init(cx: &mut gpui::App, kind: themes::ThemeKind) {
    fonts::load(cx);
    icons::init(cx);
    theme::init(cx, kind);
}
