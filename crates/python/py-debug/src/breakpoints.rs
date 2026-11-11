//! Python breakpoints as the reference types them: line breakpoints
//! (`PyLineBreakpointType`, suspending the thread by default; where they
//! can be put is [`crate::lines`]) and exception breakpoints
//! (`PyExceptionBreakpointType`, `PyExceptionBreakpointProperties`) with
//! how debugpy gets them. The reference's default exception breakpoint is
//! "Any exception" (`BaseException`) with "On termination" on and "On
//! raise" off; over DAP, "On termination" is debugpy's `uncaught` filter
//! and "On raise" its `raised` filter.
//!
//! A breakpoint for one exception class goes as DAP's `exceptionOptions`
//! (see [`options`]): debugpy ignores the filters once it gets options, so
//! the options then hold the "Any exception" breakpoint too. They carry no
//! condition and no "Ignore library files" (debugpy's is one setting of
//! the launch); without exception options a class breakpoint is not sent.

use std::path::Path;

use debugger::{
    ExceptionBreakMode, ExceptionBreakpoint, ExceptionBreakpointType, ExceptionFilter,
    ExceptionFilterSetting, ExceptionOption, ExceptionOptionSetting, LineBreakpointType,
    SuspendPolicy,
};

use crate::lines::StoppableLines;

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

/// debugpy's names in an exception option's path.
pub mod category {
    /// The category of the Python exceptions, the first path segment.
    pub const PYTHON: &str = "Python Exceptions";
}

/// The reference's titles of the two types (`PyBundle`).
pub const LINE_BREAKPOINT_TITLE: &str = "Python Line Breakpoint";
pub const EXCEPTION_BREAKPOINT_TITLE: &str = "Python Exception Breakpoint";

/// Extensions of the files line breakpoints can be put in: the Python file
/// type's (`PyLineBreakpointType.isSuitableFileType`); stubs are of
/// another type.
const PYTHON_EXTENSIONS: &[&str] = &["py", "pyw"];

/// The Python line breakpoint type: it suspends the thread by default and
/// can suspend all threads instead.
pub fn line_type() -> LineBreakpointType {
    LineBreakpointType {
        id: LINE_BREAKPOINT_TYPE.to_owned(),
        title: LINE_BREAKPOINT_TITLE.to_owned(),
        default_suspend: LINE_BREAKPOINT_SUSPEND,
        suspend_thread: true,
    }
}

/// A Python source file, going by its extension.
pub fn is_python_file(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PYTHON_EXTENSIONS.contains(&e))
}

/// The stoppable lines of a Python file, by line from line 1; `None` for
/// a file of another type.
pub fn stoppable_lines(file: &Path, text: &str) -> Option<Vec<bool>> {
    if !is_python_file(file) {
        return None;
    }
    let lines = StoppableLines::of(text);
    Some(
        (1..=lines.line_count() as u32)
            .map(|line| lines.contains(line))
            .collect(),
    )
}

/// The Python exception breakpoint type with its activation policy
/// (`PyExceptionBreakpointPropertiesPanel`).
pub fn exception_type() -> ExceptionBreakpointType {
    let option = |key: &str, label: &str| ExceptionOption {
        key: key.to_owned(),
        label: label.to_owned(),
    };
    ExceptionBreakpointType {
        id: EXCEPTION_BREAKPOINT_TYPE.to_owned(),
        title: EXCEPTION_BREAKPOINT_TITLE.to_owned(),
        any_class: ANY_EXCEPTION.to_owned(),
        any_name: ANY_EXCEPTION_NAME.to_owned(),
        options_title: "Activation policy".to_owned(),
        options: vec![
            option(property::ON_TERMINATION, "On termination"),
            option(property::ON_RAISE, "On raise"),
            option(property::IGNORE_LIBRARIES, "Ignore library files"),
        ],
        chooser_title: "Select Exception Class".to_owned(),
    }
}

/// The built-in exception classes of Python 3 (the documentation's
/// "Exception hierarchy"), for the class chooser.
pub const BUILTIN_EXCEPTIONS: &[&str] = &[
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BaseException",
    "BaseExceptionGroup",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "BytesWarning",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "DeprecationWarning",
    "EOFError",
    "EncodingWarning",
    "Exception",
    "ExceptionGroup",
    "FileExistsError",
    "FileNotFoundError",
    "FloatingPointError",
    "FutureWarning",
    "GeneratorExit",
    "ImportError",
    "ImportWarning",
    "IndentationError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PendingDeprecationWarning",
    "PermissionError",
    "ProcessLookupError",
    "PythonFinalizationError",
    "RecursionError",
    "ReferenceError",
    "ResourceWarning",
    "RuntimeError",
    "RuntimeWarning",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SyntaxWarning",
    "SystemError",
    "SystemExit",
    "TabError",
    "TimeoutError",
    "TypeError",
    "UnboundLocalError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "UnicodeTranslateError",
    "UnicodeWarning",
    "UserWarning",
    "ValueError",
    "Warning",
    "ZeroDivisionError",
];

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

/// The enabled exception breakpoints as DAP exception options, when one is
/// for a single class (filters cannot say that); `None` when the filters
/// do. "On termination" is the mode `unhandled`, "On raise" `always`; the
/// path is debugpy's category of Python exceptions, then the class (for
/// "Any exception" the category alone, which debugpy takes as
/// `BaseException`).
pub fn options(breakpoints: &[&ExceptionBreakpoint]) -> Option<Vec<ExceptionOptionSetting>> {
    let enabled = || breakpoints.iter().filter(|b| b.enabled);
    if !enabled().any(|b| b.class != ANY_EXCEPTION) {
        return None;
    }
    let mut out: Vec<ExceptionOptionSetting> = Vec::new();
    for breakpoint in enabled() {
        let mut path = vec![vec![category::PYTHON.to_owned()]];
        if breakpoint.class != ANY_EXCEPTION {
            path.push(vec![breakpoint.class.clone()]);
        }
        let wanted = [
            (property::ON_TERMINATION, ExceptionBreakMode::Unhandled),
            (property::ON_RAISE, ExceptionBreakMode::Always),
        ];
        for (flag, mode) in wanted {
            let option = ExceptionOptionSetting::new(path.clone(), mode);
            if breakpoint.flag(flag) && !out.contains(&option) {
                out.push(option);
            }
        }
    }
    Some(out)
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
    fn breakpoints_go_in_python_files_on_stoppable_lines() {
        let text = "x = 1\n\n# note\ny = 2\n";
        assert_eq!(
            stoppable_lines(Path::new("main.py"), text),
            Some(vec![true, false, false, true, false])
        );
        assert!(stoppable_lines(Path::new("app.pyw"), text).is_some());
        assert_eq!(stoppable_lines(Path::new("stubs.pyi"), text), None);
        assert_eq!(stoppable_lines(Path::new("notes.txt"), text), None);
        let line = line_type();
        assert_eq!(line.title, "Python Line Breakpoint");
        assert_eq!(line.default_suspend, SuspendPolicy::Thread);
        let exception = exception_type();
        assert_eq!(exception.title, "Python Exception Breakpoint");
        assert_eq!(exception.display_name(&any_exception()), "Any exception");
        let labels: Vec<&str> = exception.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(
            labels,
            ["On termination", "On raise", "Ignore library files"]
        );
        assert!(BUILTIN_EXCEPTIONS.windows(2).all(|w| w[0] < w[1]));
        assert!(BUILTIN_EXCEPTIONS.contains(&"KeyError"));
    }

    fn class_breakpoint(class: &str, flags: &[&str]) -> ExceptionBreakpoint {
        let mut breakpoint = ExceptionBreakpoint::new("python-exception", class);
        for flag in flags {
            breakpoint
                .properties
                .insert((*flag).to_owned(), "true".to_owned());
        }
        breakpoint
    }

    fn summary(options: &[ExceptionOptionSetting]) -> Vec<(Vec<&str>, &'static str)> {
        options
            .iter()
            .map(|o| {
                (
                    o.path.iter().flatten().map(String::as_str).collect(),
                    o.break_mode.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn options_only_for_a_class_breakpoint() {
        // "Any exception" alone is what the filters say.
        assert_eq!(options(&[&any_exception()]), None);
        let mut disabled = class_breakpoint("ValueError", &[property::ON_RAISE]);
        disabled.enabled = false;
        assert_eq!(options(&[&any_exception(), &disabled]), None);
        assert_eq!(options(&[]), None);
    }

    #[test]
    fn a_class_breakpoint_goes_with_the_any_exception_one() {
        let value_error = class_breakpoint("ValueError", &[property::ON_RAISE]);
        let sent = options(&[&any_exception(), &value_error]).unwrap();
        assert_eq!(
            summary(&sent),
            [
                (vec!["Python Exceptions"], "unhandled"),
                (vec!["Python Exceptions", "ValueError"], "always"),
            ]
        );
    }

    #[test]
    fn modes_follow_on_termination_and_on_raise() {
        let both = class_breakpoint("KeyError", &[property::ON_TERMINATION, property::ON_RAISE]);
        let none = class_breakpoint("TypeError", &[]);
        let sent = options(&[&both, &none]).unwrap();
        assert_eq!(
            summary(&sent),
            [
                (vec!["Python Exceptions", "KeyError"], "unhandled"),
                (vec!["Python Exceptions", "KeyError"], "always"),
            ]
        );
        // A class breakpoint with neither sends nothing: the adapter then
        // breaks on nothing.
        assert!(options(&[&none]).unwrap().is_empty());
    }

    #[test]
    fn two_breakpoints_share_an_option_once() {
        let a = class_breakpoint("ValueError", &[property::ON_RAISE]);
        let b = class_breakpoint("ValueError", &[property::ON_RAISE]);
        assert_eq!(options(&[&a, &b]).unwrap().len(), 1);
    }

    #[test]
    fn two_breakpoints_share_a_filter_once() {
        let a = any_exception();
        let b = any_exception();
        assert_eq!(ids(&filters(&[&a, &b], &debugpy())), ["uncaught"]);
    }
}
