//! The breakpoint icons: the New UI icons of the IntelliJ Platform
//! Community repository (`platform/icons/src/expui/breakpoints`,
//! Apache-2.0, see NOTICE), a light and a dark variant each. The question
//! badge of conditional breakpoints is drawn over a breakpoint's icon
//! (`editor::GutterIcon::badge`); its file is set in a square box.

use ui::icons::{IconName, IconSvg};

/// A line breakpoint (`AllIcons.Debugger.Db_set_breakpoint`).
pub const BREAKPOINT: IconName = IconName("breakpoints/breakpoint");
pub const DISABLED: IconName = IconName("breakpoints/breakpointDisabled");
pub const MUTED: IconName = IconName("breakpoints/breakpointMuted");
pub const MUTED_DISABLED: IconName = IconName("breakpoints/breakpointMutedDisabled");
/// A breakpoint that does not suspend (`Db_no_suspend_breakpoint`).
pub const UNSUSPENDENT: IconName = IconName("breakpoints/breakpointUnsuspendent");
/// Breakpoints the running debugger verified (`Db_verified_breakpoint`,
/// `Db_verified_no_suspend_breakpoint`).
pub const VALID: IconName = IconName("breakpoints/breakpointValid");
pub const UNSUSPENDENT_VALID: IconName = IconName("breakpoints/breakpointUnsuspendentValid");
/// A breakpoint the running debugger could not set (`Db_invalid_breakpoint`).
pub const INVALID: IconName = IconName("breakpoints/breakpointInvalid");
/// Exception breakpoints (`Db_exception_breakpoint`).
pub const EXCEPTION: IconName = IconName("breakpoints/breakpointException");
pub const EXCEPTION_DISABLED: IconName = IconName("breakpoints/breakpointExceptionDisabled");
/// The badge of a conditional breakpoint (`AllIcons.Debugger.Question_badge`).
pub const QUESTION_BADGE: IconName = IconName("breakpoints/questionBadge");

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[IconSvg] = &[
            $(IconSvg {
                name: concat!("breakpoints/", $name),
                light: include_bytes!(concat!("../assets/icons/breakpoints/", $name, ".svg")),
                dark: Some(include_bytes!(concat!("../assets/icons/breakpoints/", $name, "_dark.svg"))),
            },)*
        ];
    };
}

icons![
    "breakpoint",
    "breakpointDisabled",
    "breakpointMuted",
    "breakpointMutedDisabled",
    "breakpointUnsuspendent",
    "breakpointUnsuspendentValid",
    "breakpointValid",
    "breakpointInvalid",
    "breakpointException",
    "breakpointExceptionDisabled",
    "questionBadge",
];

/// Registers the icons (the platform's light-only breakpoint icon gets its
/// dark variant).
pub fn register(cx: &mut gpui::App) {
    ui::icons::register(cx, ICONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_rasterizes_in_both_variants() {
        for icon in ICONS {
            assert!(
                ui::icons::rasterize(icon.light, 28).is_some(),
                "{}",
                icon.name
            );
            assert!(
                ui::icons::rasterize(icon.dark.unwrap(), 28).is_some(),
                "{}",
                icon.name
            );
        }
        let names: Vec<&str> = ICONS.iter().map(|i| i.name).collect();
        for icon in [
            BREAKPOINT,
            DISABLED,
            MUTED,
            MUTED_DISABLED,
            UNSUSPENDENT,
            VALID,
            UNSUSPENDENT_VALID,
            INVALID,
            EXCEPTION,
            EXCEPTION_DISABLED,
            QUESTION_BADGE,
        ] {
            assert!(names.contains(&icon.0), "{}", icon.0);
        }
    }
}
