//! Python breakpoints as the reference types them: line breakpoints
//! (`PyLineBreakpointType`, suspending the thread by default; where they
//! can be put is [`crate::lines`]) and exception breakpoints
//! (`PyExceptionBreakpointType`, `PyExceptionBreakpointProperties`) with
//! how debugpy gets them. The reference's default exception breakpoint is
//! "Any exception" (`BaseException`) with "On termination" on and "On
//! raise" off; over DAP, "On termination" is debugpy's `uncaught` filter
//! and "On raise" its `raised` filter.
//!
//! A breakpoint for one exception class would need DAP's
//! `exceptionOptions`, which the client does not send yet: only "Any
//! exception" breakpoints reach the adapter.

use debugger::{ExceptionBreakpoint, ExceptionFilter, ExceptionFilterSetting, SuspendPolicy};

/// The reference's type of Python line breakpoints.
pub const LINE_BREAKPOINT_TYPE: &str = "python-line";
/// What a new Python line breakpoint suspends
/// (`PyLineBreakpointType.getDefaultSuspendPolicy`).
pub const LINE_BREAKPOINT_SUSPEND: SuspendPolicy = SuspendPolicy::Thread;
/// The reference's type of Python exception breakpoints.
pub const EXCEPTION_BREAKPOINT_TYPE: &str = "python-exception";
/// The class of the "Any exception" breakpoint.
pub const ANY_EXCEPTION: &str = "BaseException";
/// The reference's name of the "Any exception" breakpoint
/// (`debugger.exception.breakpoint.any.exception`).
pub const ANY_EXCEPTION_NAME: &str = "Any exception";

/// The properties of a Python exception breakpoint, under the reference's
/// attribute names.
pub mod property {
    /// The class (`exception`), held by [`debugger::ExceptionBreakpoint::class`].
    pub const EXCEPTION: &str = "exception";
    /// "On termination".
    pub const ON_TERMINATION: &str = "notifyOnTerminate";
    /// "On raise".
    pub const ON_RAISE: &str = "notifyOnlyOnFirst";
    /// "Ignore library files".
    pub const IGNORE_LIBRARIES: &str = "ignoreLibraries";
}

/// debugpy's filters.
pub mod filter {
    /// Breaks when the process is exiting due to an unhandled exception.
    pub const UNCAUGHT: &str = "uncaught";
    /// Breaks whenever an exception is raised.
    pub const RAISED: &str = "raised";
}

/// The default exception breakpoint of a project ("Any exception", On
/// termination), as the reference creates it.
pub fn any_exception() -> ExceptionBreakpoint {
    let mut breakpoint = ExceptionBreakpoint::new(EXCEPTION_BREAKPOINT_TYPE, ANY_EXCEPTION);
    breakpoint
        .properties
        .insert(property::ON_TERMINATION.to_owned(), "true".to_owned());
    breakpoint
}

/// The name of a breakpoint as the reference shows it: "Any exception"
/// for `BaseException`, else the class.
pub fn display_name(breakpoint: &ExceptionBreakpoint) -> &str {
    if breakpoint.class == ANY_EXCEPTION {
        ANY_EXCEPTION_NAME
    } else {
        &breakpoint.class
    }
}

/// The filters for the enabled exception breakpoints: "On termination" of
/// an "Any exception" breakpoint turns on `uncaught`, "On raise" turns on
/// `raised`; a condition goes with them where the adapter takes one.
/// Filters the adapter does not list are left out (when it lists any).
pub fn filters(
    breakpoints: &[&ExceptionBreakpoint],
    available: &[ExceptionFilter],
) -> Vec<ExceptionFilterSetting> {
    let mut out: Vec<ExceptionFilterSetting> = Vec::new();
    for breakpoint in breakpoints {
        if !breakpoint.enabled || breakpoint.class != ANY_EXCEPTION {
            continue;
        }
        let wanted = [
            (property::ON_TERMINATION, filter::UNCAUGHT),
            (property::ON_RAISE, filter::RAISED),
        ];
        for (flag, id) in wanted {
            if !breakpoint.flag(flag) || out.iter().any(|f| f.id == id) {
                continue;
            }
            let offered = available.iter().find(|f| f.id == id);
            if offered.is_none() && !available.is_empty() {
                continue;
            }
            let mut setting = ExceptionFilterSetting::new(id);
            if offered.is_some_and(|f| f.supports_condition) {
                setting.condition = breakpoint.condition.clone();
            }
            out.push(setting);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// debugpy's filters (`initialize` of `adapter/clients.py`).
    fn debugpy() -> Vec<ExceptionFilter> {
        ["raised", "uncaught", "userUnhandled"]
            .into_iter()
            .map(|id| ExceptionFilter {
                id: id.to_owned(),
                label: id.to_owned(),
                default: id == "uncaught",
                ..ExceptionFilter::default()
            })
            .collect()
    }

    fn ids(settings: &[ExceptionFilterSetting]) -> Vec<&str> {
        settings.iter().map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn any_exception_on_termination_is_uncaught() {
        let any = any_exception();
        assert_eq!(any.kind, "python-exception");
        assert_eq!(display_name(&any), "Any exception");
        assert_eq!(ids(&filters(&[&any], &debugpy())), ["uncaught"]);
    }

    #[test]
    fn on_raise_is_raised() {
        let mut any = any_exception();
        any.properties
            .insert(property::ON_RAISE.into(), "true".into());
        assert_eq!(ids(&filters(&[&any], &debugpy())), ["uncaught", "raised"]);
        any.properties
            .insert(property::ON_TERMINATION.into(), "false".into());
        assert_eq!(ids(&filters(&[&any], &debugpy())), ["raised"]);
        // Neither: nothing breaks.
        any.properties.clear();
        assert!(filters(&[&any], &debugpy()).is_empty());
    }

    #[test]
    fn disabled_and_class_breakpoints_are_not_sent() {
        let mut any = any_exception();
        any.enabled = false;
        let mut value_error = ExceptionBreakpoint::new("python-exception", "ValueError");
        value_error
            .properties
            .insert(property::ON_RAISE.into(), "true".into());
        assert_eq!(display_name(&value_error), "ValueError");
        assert!(filters(&[&any, &value_error], &debugpy()).is_empty());
    }

    #[test]
    fn conditions_go_where_the_adapter_takes_them() {
        let mut any = any_exception();
        any.condition = Some("x > 1".into());
        // debugpy's filters take no condition.
        assert_eq!(filters(&[&any], &debugpy())[0].condition, None);
        let mut with_condition = debugpy();
        for f in &mut with_condition {
            f.supports_condition = true;
        }
        assert_eq!(
            filters(&[&any], &with_condition)[0].condition.as_deref(),
            Some("x > 1")
        );
        // An adapter without `uncaught` gets nothing for it; one that
        // lists no filters gets the ids.
        let raised_only: Vec<ExceptionFilter> =
            debugpy().into_iter().filter(|f| f.id == "raised").collect();
        assert!(filters(&[&any], &raised_only).is_empty());
        assert_eq!(ids(&filters(&[&any], &[])), ["uncaught"]);
    }

    #[test]
    fn two_breakpoints_share_a_filter_once() {
        let a = any_exception();
        let b = any_exception();
        assert_eq!(ids(&filters(&[&a, &b], &debugpy())), ["uncaught"]);
    }
}
