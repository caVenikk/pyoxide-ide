//! The icon colour palette: the colours the platform icons are drawn with
//! and the roles that replace them.
//!
//! The New UI icons use a small set of colours (`Gray.Stroke`,
//! `Green.Fill`, ...). A UI theme recolours icons by mapping these palette
//! keys to its own colours (`icons` > `ColorPalette`), and the reference
//! replaces every occurrence of the key's colour in an icon by the theme's
//! (`UITheme.toColorString`, `UiThemePaletteScopeManager`). Here every key
//! is a [`UiRole`] of the "Icons" group: a theme whose role differs from
//! the key's colour has its icons recoloured ([`Theme::icon_recolors`]).
//!
//! The colours are those of `UITheme.kt` of IntelliJ Platform Community
//! (Apache-2.0, see NOTICE): `colorPaletteNewLight`, `colorPaletteNewDark`
//! and, for check boxes of classic themes, `colorPaletteClassic`; the New
//! UI check box colours are those of the check box icons of the New UI
//! themes (`themes/expUI/icons/dark/`).

use crate::color::Rgba;
use crate::model::{Controls, Theme, ThemeKind};
use crate::roles::UiRole;

/// The palette key of every icon role.
pub const ICON_ROLES: &[(UiRole, &str)] = &[
    (UiRole::IconGrayFill, "Gray.Fill"),
    (UiRole::IconGraySecondaryStroke, "Gray.SecondaryStroke"),
    (UiRole::IconGrayStroke, "Gray.Stroke"),
    (UiRole::IconBlueFill, "Blue.Fill"),
    (UiRole::IconBlueSolid, "Blue.Solid"),
    (UiRole::IconBlueStroke, "Blue.Stroke"),
    (UiRole::IconGreenFill, "Green.Fill"),
    (UiRole::IconGreenSolid, "Green.Solid"),
    (UiRole::IconGreenStroke, "Green.Stroke"),
    (UiRole::IconRedFill, "Red.Fill"),
    (UiRole::IconRedSolid, "Red.Solid"),
    (UiRole::IconRedStroke, "Red.Stroke"),
    (UiRole::IconYellowFill, "Yellow.Fill"),
    (UiRole::IconYellowSolid, "Yellow.Solid"),
    (UiRole::IconYellowStroke, "Yellow.Stroke"),
    (UiRole::IconOrangeFill, "Orange.Fill"),
    (UiRole::IconOrangeStroke, "Orange.Stroke"),
    (UiRole::IconPurpleFill, "Purple.Fill"),
    (UiRole::IconPurpleStroke, "Purple.Stroke"),
    (UiRole::IconTealFill, "Teal.Fill"),
    (UiRole::IconTealStroke, "Teal.Stroke"),
];

/// `colorPaletteNewLight`.
const NEW_LIGHT: &[(&str, u32)] = &[
    ("Gray.Fill", 0xEBECF0),
    ("Gray.SecondaryStroke", 0xA8ADBD),
    ("Gray.Stroke", 0x6C707E),
    ("Blue.Fill", 0xEDF3FF),
    ("Blue.Solid", 0x4682FA),
    ("Blue.Stroke", 0x3574F0),
    ("Green.Fill", 0xF2FCF3),
    ("Green.Solid", 0x55A76A),
    ("Green.Stroke", 0x208A3C),
    ("Red.Fill", 0xFFF7F7),
    ("Red.Solid", 0xE55765),
    ("Red.Stroke", 0xDB3B4B),
    ("Yellow.Fill", 0xFFFAEB),
    ("Yellow.Solid", 0xFFAF0F),
    ("Yellow.Stroke", 0xC27D04),
    ("Orange.Fill", 0xFFF4EB),
    ("Orange.Stroke", 0xE56D17),
    ("Purple.Fill", 0xFAF5FF),
    ("Purple.Stroke", 0x834DF0),
    ("Teal.Fill", 0xF2FCFC),
    ("Teal.Stroke", 0x039BA1),
];

/// `colorPaletteNewDark` (it has no `Blue.Solid`, `Green.Solid`,
/// `Red.Solid`: these keys recolour nothing in a dark theme).
const NEW_DARK: &[(&str, u32)] = &[
    ("Gray.Fill", 0x43454A),
    ("Gray.SecondaryStroke", 0x6F737A),
    ("Gray.Stroke", 0xCED0D6),
    ("Blue.Fill", 0x25324D),
    ("Blue.Stroke", 0x548AF7),
    ("Green.Fill", 0x253627),
    ("Green.Stroke", 0x57965C),
    ("Red.Fill", 0x402929),
    ("Red.Stroke", 0xDB5C5C),
    ("Yellow.Fill", 0x3D3223),
    ("Yellow.Solid", 0xF2C55C),
    ("Yellow.Stroke", 0xD6AE58),
    ("Orange.Fill", 0x45322B),
    ("Orange.Stroke", 0xC77D55),
    ("Purple.Fill", 0x2F2936),
    ("Purple.Stroke", 0xA571E6),
    ("Teal.Fill", 0x1D3838),
    ("Teal.Stroke", 0x24A394),
];

/// The colour a palette key stands for in the icons of a theme of `kind`
/// (`UITheme.toColorString` for the keys of the New UI palette).
pub fn reference_color(key: &str, kind: ThemeKind) -> Option<Rgba> {
    let table = match kind {
        ThemeKind::Light => NEW_LIGHT,
        ThemeKind::Dark => NEW_DARK,
    };
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, rgb)| rgb_of(*rgb))
}

fn rgb_of(rgb: u32) -> Rgba {
    Rgba::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The colours of check boxes a theme gets when it does not set them, by
/// the look of its controls: those of the check box icons of the classic
/// themes (`colorPaletteClassic`, `.Dark` for dark themes) or of the New
/// UI themes.
pub fn control_default(role: UiRole, controls: Controls, kind: ThemeKind) -> Option<Rgba> {
    use UiRole::*;
    let dark = kind == ThemeKind::Dark;
    let rgb = match (controls, dark) {
        (Controls::Classic, false) => match role {
            CheckboxBackground => 0xFFFFFF,
            CheckboxBorder => 0xB0B0B0,
            CheckboxBackgroundSelected => 0x4F9EE3,
            CheckboxBorderSelected => 0x4B97D9,
            CheckboxCheck => 0xFEFEFE,
            CheckboxBackgroundDisabled => 0xF2F2F2,
            CheckboxBorderDisabled => 0xBDBDBD,
            CheckboxCheckDisabled => 0xABABAB,
            _ => return None,
        },
        (Controls::Classic, true) => match role {
            CheckboxBackground => 0x43494A,
            CheckboxBorder => 0x6B6B6B,
            CheckboxBackgroundSelected => 0x43494A,
            CheckboxBorderSelected => 0x6B6B6B,
            CheckboxCheck => 0xA7A7A7,
            CheckboxBackgroundDisabled => 0x3C3F41,
            CheckboxBorderDisabled => 0x545556,
            CheckboxCheckDisabled => 0x606060,
            _ => return None,
        },
        (Controls::New, false) => match role {
            CheckboxBackground => 0xFFFFFF,
            CheckboxBorder => 0xA8ADBD,
            CheckboxBackgroundSelected => 0x3574F0,
            CheckboxBorderSelected => 0x3574F0,
            CheckboxCheck => 0xFFFFFF,
            CheckboxBackgroundDisabled => 0xF7F8FA,
            CheckboxBorderDisabled => 0xDFE1E5,
            CheckboxCheckDisabled => 0xC9CCD6,
            _ => return None,
        },
        (Controls::New, true) => match role {
            CheckboxBackground => 0x2B2D30,
            CheckboxBorder => 0x5A5D63,
            CheckboxBackgroundSelected => 0x3574F0,
            CheckboxBorderSelected => 0x3574F0,
            CheckboxCheck => 0xFFFFFF,
            CheckboxBackgroundDisabled => 0x393B40,
            CheckboxBorderDisabled => 0x5A5D63,
            CheckboxCheckDisabled => 0x6F737A,
            _ => return None,
        },
    };
    Some(rgb_of(rgb))
}

impl Theme {
    /// The icon colours this theme replaces: the colour a palette key
    /// stands for in the icons, and the theme's colour for it. Empty for a
    /// theme that keeps the icons' own colours.
    pub fn icon_recolors(&self) -> Vec<(Rgba, Rgba)> {
        ICON_ROLES
            .iter()
            .filter_map(|&(role, key)| {
                let from = reference_color(key, self.kind)?;
                let to = self.ui[role];
                (to != from).then_some((from, to))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_role_has_a_key_and_the_keys_are_known() {
        let icons: Vec<UiRole> = UiRole::ALL
            .iter()
            .copied()
            .filter(|r| r.group() == "Icons")
            .collect();
        assert_eq!(icons.len(), ICON_ROLES.len());
        for (role, key) in ICON_ROLES {
            assert!(icons.contains(role), "{role}");
            assert!(reference_color(key, ThemeKind::Light).is_some(), "{key}");
        }
        assert_eq!(
            reference_color("Gray.Stroke", ThemeKind::Dark),
            Some(Rgba::rgb(0xCE, 0xD0, 0xD6))
        );
        assert_eq!(reference_color("Blue.Solid", ThemeKind::Dark), None);
    }

    #[test]
    fn the_base_themes_keep_the_icons_own_colours() {
        for theme in [crate::builtin::dark(), crate::builtin::light()] {
            assert_eq!(theme.icon_recolors(), Vec::new(), "{}", theme.id);
        }
    }

    #[test]
    fn a_changed_icon_role_recolours_its_key() {
        let mut theme = crate::builtin::dark();
        theme.ui[UiRole::IconGreenStroke] = Rgba::rgb(0xA6, 0xE2, 0x2E);
        assert_eq!(
            theme.icon_recolors(),
            vec![(Rgba::rgb(0x57, 0x96, 0x5C), Rgba::rgb(0xA6, 0xE2, 0x2E))]
        );
    }

    #[test]
    fn control_defaults_cover_check_boxes() {
        for controls in [Controls::New, Controls::Classic] {
            for kind in [ThemeKind::Dark, ThemeKind::Light] {
                for role in UiRole::ALL.iter().filter(|r| r.group() == "Controls") {
                    assert!(control_default(*role, controls, kind).is_some(), "{role}");
                }
                assert_eq!(control_default(UiRole::Accent, controls, kind), None);
            }
        }
    }
}
