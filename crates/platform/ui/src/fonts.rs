//! Fonts that ship with the IDE: Inter for the interface and JetBrains Mono
//! for the editor, both under the SIL Open Font License 1.1 (license texts
//! next to the files in `assets/fonts/`).

use std::borrow::Cow;

use gpui::App;

/// Family name of the interface font as stored in the font files.
pub const UI_FAMILY: &str = "Inter";
/// Family name of the editor font as stored in the font files.
pub const MONO_FAMILY: &str = "JetBrains Mono";

const FILES: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-BoldItalic.ttf"),
];

/// Registers the bundled fonts with the text system.
pub fn load(cx: &mut App) {
    cx.text_system()
        .add_fonts(FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("the bundled fonts are valid");
}

/// The first family of `families` that the text system knows, else the
/// first one (the platform then picks its fallback).
pub fn pick_family(families: &[String], available: &[String]) -> String {
    families
        .iter()
        .find(|f| available.iter().any(|a| a == *f))
        .or_else(|| families.first())
        .cloned()
        .unwrap_or_else(|| UI_FAMILY.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_first_available_family() {
        let families = vec!["Nope".to_owned(), "Inter".to_owned(), "Arial".to_owned()];
        let available = vec!["Arial".to_owned(), "Inter".to_owned()];
        assert_eq!(pick_family(&families, &available), "Inter");
        assert_eq!(pick_family(&families, &[]), "Nope");
        assert_eq!(pick_family(&[], &[]), UI_FAMILY);
    }
}
