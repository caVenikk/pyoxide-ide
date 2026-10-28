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

/// The debugger's actions (`platform/icons/src/expui/run`, `debugger` and
/// `general`): Rerun of a debug session (`AllIcons.Actions.RestartDebugger`;
/// its outline on the run widget's green slot), Resume, Pause, the steps,
/// View and Mute Breakpoints, Run to Cursor, Show Execution Point and the
/// layout button.
pub const RERUN_DEBUG: IconName = IconName("debug/restartDebug");
pub const RERUN_DEBUG_STROKE: IconName = IconName("debug/restartDebug_stroke");
pub const RESUME: IconName = IconName("debug/resume");
pub const RESUME_STROKE: IconName = IconName("debug/resume_stroke");
pub const PAUSE: IconName = IconName("debug/pause");
pub const STEP_OVER: IconName = IconName("debug/stepOver");
pub const STEP_INTO: IconName = IconName("debug/stepInto");
pub const STEP_INTO_MY_CODE: IconName = IconName("debug/stepIntoMyCode");
pub const STEP_OUT: IconName = IconName("debug/stepOut");
pub const VIEW_BREAKPOINTS: IconName = IconName("debug/viewBreakpoints");
pub const MUTE_BREAKPOINTS: IconName = IconName("debug/muteBreakpoints");
pub const RUN_TO_CURSOR: IconName = IconName("debug/runToCursor");
pub const FORCE_RUN_TO_CURSOR: IconName = IconName("debug/forceRunToCursor");
pub const SHOW_EXECUTION_POINT: IconName = IconName("debug/showCurrentFrame");
pub const LAYOUT: IconName = IconName("debug/layout");

macro_rules! action_icons {
    ($($name:literal),* $(,)?) => {
        const ACTION_ICONS: &[IconSvg] = &[
            $(IconSvg {
                name: concat!("debug/", $name),
                light: include_bytes!(concat!("../assets/icons/debug/", $name, ".svg")),
                dark: Some(include_bytes!(concat!("../assets/icons/debug/", $name, "_dark.svg"))),
            },)*
            // Outlines tinted where they are drawn: one variant.
            IconSvg {
                name: "debug/restartDebug_stroke",
                light: include_bytes!("../assets/icons/debug/restartDebug_stroke.svg"),
                dark: None,
            },
            IconSvg {
                name: "debug/resume_stroke",
                light: include_bytes!("../assets/icons/debug/resume_stroke.svg"),
                dark: None,
            },
        ];
    };
}

action_icons![
    "restartDebug",
    "resume",
    "pause",
    "stepOver",
    "stepInto",
    "stepIntoMyCode",
    "stepOut",
    "viewBreakpoints",
    "muteBreakpoints",
    "runToCursor",
    "forceRunToCursor",
    "showCurrentFrame",
    "layout",
];

/// The frames and variables views (`platform/icons/src/expui/debugger`):
/// a frame (`AllIcons.Debugger.Frame`), the current thread
/// (`ThreadCurrent`), values (`Db_primitive`, `Db_array`, `Value`), the
/// Special Variables and Return Values groups (`SpecialVar`,
/// `WatchLastReturnValue`), watches (`Watch`, `AddToWatch`) and the
/// evaluate field's result (`run/evaluateExpression`).
pub const FRAME: IconName = IconName("values/frame");
pub const THREAD_CURRENT: IconName = IconName("values/threadCurrent");
pub const PRIMITIVE: IconName = IconName("values/dbPrimitive");
pub const ARRAY: IconName = IconName("values/dbArray");
pub const OBJECT: IconName = IconName("values/value");
pub const SPECIAL_VARIABLES: IconName = IconName("values/specialVar");
/// The Protected Attributes group (`AllIcons.Nodes.C_protected`).
pub const PROTECTED: IconName = IconName("values/accessProtected");
pub const RETURN_VALUES: IconName = IconName("values/watchLastReturnValue");
pub const WATCH: IconName = IconName("values/watch");
pub const ADD_TO_WATCHES: IconName = IconName("values/addToWatch");
/// The evaluate field's result (`AllIcons.Debugger.Db_evaluateNode`, the
/// New UI's `run/evaluateExpression`) and Evaluate Expression.
pub const EVALUATION_RESULT: IconName = IconName("values/evaluateExpression");
pub const EVALUATE_EXPRESSION: IconName = EVALUATION_RESULT;
/// Documentation and Problems of the value under the mouse
/// (`AllIcons.Ide.FatalErrorRead`, the New UI's `status/errorOutline`).
pub const DOCUMENTATION_AND_PROBLEMS: IconName = IconName("values/errorOutline");

macro_rules! value_icons {
    ($($name:literal),* $(,)?) => {
        const VALUE_ICONS: &[IconSvg] = &[
            $(IconSvg {
                name: concat!("values/", $name),
                light: include_bytes!(concat!("../assets/icons/values/", $name, ".svg")),
                dark: Some(include_bytes!(concat!("../assets/icons/values/", $name, "_dark.svg"))),
            },)*
        ];
    };
}

value_icons![
    "frame",
    "threadCurrent",
    "dbPrimitive",
    "dbArray",
    "value",
    "specialVar",
    "accessProtected",
    "watchLastReturnValue",
    "watch",
    "addToWatch",
    "evaluateExpression",
    "errorOutline",
];

/// Registers the icons (the platform's light-only breakpoint icon gets its
/// dark variant).
pub fn register(cx: &mut gpui::App) {
    ui::icons::register(cx, ICONS);
    ui::icons::register(cx, ACTION_ICONS);
    ui::icons::register(cx, VALUE_ICONS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_icon_rasterizes() {
        for icon in ACTION_ICONS {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            if let Some(dark) = icon.dark {
                assert!(ui::icons::rasterize(dark, 32).is_some(), "{}", icon.name);
            }
        }
        let names: Vec<&str> = ACTION_ICONS.iter().map(|i| i.name).collect();
        for icon in [
            RERUN_DEBUG,
            RERUN_DEBUG_STROKE,
            RESUME,
            RESUME_STROKE,
            PAUSE,
            STEP_OVER,
            STEP_INTO,
            STEP_INTO_MY_CODE,
            STEP_OUT,
            VIEW_BREAKPOINTS,
            MUTE_BREAKPOINTS,
            RUN_TO_CURSOR,
            FORCE_RUN_TO_CURSOR,
            SHOW_EXECUTION_POINT,
            LAYOUT,
        ] {
            assert!(names.contains(&icon.0), "{}", icon.0);
        }
    }

    #[test]
    fn every_value_icon_rasterizes_in_both_variants() {
        for icon in VALUE_ICONS {
            assert!(
                ui::icons::rasterize(icon.light, 32).is_some(),
                "{}",
                icon.name
            );
            assert!(
                ui::icons::rasterize(icon.dark.unwrap(), 32).is_some(),
                "{}",
                icon.name
            );
        }
        let names: Vec<&str> = VALUE_ICONS.iter().map(|i| i.name).collect();
        for icon in [
            FRAME,
            THREAD_CURRENT,
            PRIMITIVE,
            ARRAY,
            OBJECT,
            SPECIAL_VARIABLES,
            RETURN_VALUES,
            WATCH,
            ADD_TO_WATCHES,
            EVALUATION_RESULT,
        ] {
            assert!(names.contains(&icon.0), "{}", icon.0);
        }
    }

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
