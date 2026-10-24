//! Quick documentation of Python modules as the reference shows it: the
//! header `Module contextlib` (`Package json` for a package), the
//! docstring, the file's facts and, for a module of the standard library,
//! a link to its page on docs.python.org (IntelliJ Community,
//! `PyDocumentationBuilder.addModulePath`, `PythonDocumentationProvider.
//! getOnlyUrlFor`, `PyStdlibDocumentationLinkProvider`; see
//! notices/python-module-documentation.md).
//!
//! The type server's hover says that a name is a module: ty sends
//! `<module 'contextlib'>`, basedpyright `(module) contextlib`, as the
//! declaration code block; the declaration is the module's file.

use std::path::{Component, Path, PathBuf};

use code_intel::{Markup, MarkupKind};
use code_intel_ui::file_doc::FileDoc;

/// The name a type server's hover gives a module (`contextlib` of
/// ```` ```xml\n<module 'contextlib'>\n``` ````, `json` of
/// ```` ```python\n(module) json\n``` ````); `None` when the hover is not
/// about a module.
pub fn hovered_module(markup: &Markup) -> Option<String> {
    let text = match markup.kind {
        MarkupKind::Markdown => {
            let rest = markup.text.trim_start().strip_prefix("```")?;
            let (_, code) = rest.split_once('\n')?;
            code.split("```")
                .next()?
                .trim()
                .lines()
                .next()?
                .trim()
                .to_owned()
        }
        MarkupKind::PlainText => markup.text.trim().lines().next()?.trim().to_owned(),
    };
    let name = if let Some(rest) = text.strip_prefix("<module '") {
        rest.strip_suffix("'>")?
    } else {
        text.strip_prefix("(module) ")?
    };
    let valid = !name.is_empty()
        && name
            .split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '_'));
    valid.then(|| name.to_owned())
}

/// A file of the standard library: in a typeshed `stdlib` folder (the
/// stubs ty and basedpyright bring) or in an interpreter's library folder
/// (`lib/python3.13/`, `Lib\` on Windows) outside `site-packages`
/// (`PySkeletonUtil.isStdLib`). Blocking on Windows layouts.
pub fn is_stdlib(path: &Path) -> bool {
    let names: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if names
        .iter()
        .any(|n| n == "site-packages" || n == "dist-packages")
    {
        return false;
    }
    let typeshed = names.iter().position(|n| n.starts_with("typeshed"));
    if let Some(at) = typeshed
        && names[at..].iter().any(|n| n == "stdlib")
    {
        return true;
    }
    // `lib/python3.13/` of a Unix installation; `Lib\` next to
    // `python.exe` on Windows (blocking: looks for it).
    path.ancestors().any(|dir| {
        if !dir
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("lib"))
        {
            return false;
        }
        let next = path
            .strip_prefix(dir)
            .ok()
            .and_then(|rest| rest.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned());
        let versioned = next.is_some_and(|n| {
            n.strip_prefix("python")
                .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
        });
        versioned || dir.parent().is_some_and(|p| p.join("python.exe").is_file())
    })
}

/// The `major.minor` of a version (`3.13` of `3.13.1`;
/// `PythonDocumentationProvider.pyVersion`).
pub fn major_minor(version: &str) -> Option<String> {
    let mut parts = version.split('.');
    let major = parts.next()?.trim();
    let minor = parts.next().unwrap_or("").trim();
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !digits(major) {
        return None;
    }
    Some(if digits(minor) {
        format!("{major}.{minor}")
    } else {
        major.to_owned()
    })
}

/// The page of a standard library module on docs.python.org
/// (`PyStdlibDocumentationLinkProvider.getStdlibUrlFor` for a module): the
/// library index of the interpreter's version, the module's page when it
/// has one (`https://docs.python.org/3.13/library/contextlib.html`).
pub fn stdlib_url(module: &str, version: Option<&str>) -> String {
    let version = version.and_then(major_minor);
    let mut url = String::from("https://docs.python.org/");
    if let Some(v) = &version {
        url.push_str(v);
        url.push('/');
    }
    let old = version
        .as_deref()
        .is_some_and(|v| v.starts_with("2.4") || v.starts_with("2.5"));
    url.push_str(if old { "lib/" } else { "library/" });
    // The pages of Python 2 are not listed; its modules get the index.
    let py3 = version.as_deref().is_none_or(|v| v.starts_with('3'));
    if py3 && let Some(page) = web_page(module) {
        url.push_str(page);
        url.push_str(".html");
    }
    url
}

/// The page of a module (`webPageName`, `getModuleName`).
fn web_page(module: &str) -> Option<&'static str> {
    let module = match module {
        "cPickle" => "pickle",
        "pyexpat" => "xml.parsers.expat",
        other => other,
    };
    if let Some((_, page)) = OTHER_PAGE.iter().find(|(m, _)| *m == module) {
        return Some(page);
    }
    SAME_PAGE.iter().find(|m| **m == module).copied()
}

/// The documentation of a module the hover is about: its kind and name,
/// the file type, and for the standard library the docs.python.org link
/// for the interpreter `version` (no link without an interpreter, as the
/// reference needs an SDK for one).
pub fn module_doc(path: &Path, module: String, interpreter: Option<Option<String>>) -> FileDoc {
    let package = path.file_stem().is_some_and(|s| s == "__init__");
    let stub = path.extension().is_some_and(|e| e == "pyi");
    let external_url = interpreter
        .filter(|_| is_stdlib(path))
        .map(|version| stdlib_url(&module, version.as_deref().filter(|v| !v.is_empty())));
    FileDoc {
        kind: if package { "Package" } else { "Module" }.to_owned(),
        name: module,
        // The display names of the reference's file types.
        file_type: if stub { "PythonStub" } else { "Python" }.to_owned(),
        external_url,
    }
}

/// The dotted name of the module at `path`, as the header names it: from
/// the project's roots and the packages around it, else the name the
/// hover gave.
pub fn module_name(path: &Path, roots: &[PathBuf], hovered: String) -> String {
    crate::services::module_name(path, roots).unwrap_or(hovered)
}

/// The standard library modules with a page of their own, named as the
/// module (`py3LibraryModulesToWebpageName` of the reference).
const SAME_PAGE: &[&str] = &[
    "__future__",
    "__main__",
    "_thread",
    "abc",
    "aifc",
    "argparse",
    "array",
    "ast",
    "asyncio",
    "atexit",
    "audioop",
    "base64",
    "bdb",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "calendar",
    "cgi",
    "cgitb",
    "chunk",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "collections.abc",
    "colorsys",
    "compileall",
    "concurrent.futures",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "crypt",
    "csv",
    "ctypes",
    "curses",
    "curses.ascii",
    "curses.panel",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "doctest",
    "email",
    "email.charset",
    "email.contentmanager",
    "email.encoders",
    "email.errors",
    "email.generator",
    "email.header",
    "email.headerregistry",
    "email.iterators",
    "email.message",
    "email.mime",
    "email.parser",
    "email.policy",
    "email.utils",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "html.entities",
    "html.parser",
    "http",
    "http.client",
    "http.cookiejar",
    "http.cookies",
    "http.server",
    "imaplib",
    "imghdr",
    "importlib",
    "importlib.resources",
    "importlib.resources.abc",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "linecache",
    "locale",
    "logging",
    "logging.config",
    "logging.handlers",
    "lzma",
    "mailbox",
    "mailcap",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "msilib",
    "msvcrt",
    "multiprocessing",
    "multiprocessing.shared_memory",
    "netrc",
    "nis",
    "nntplib",
    "numbers",
    "operator",
    "optparse",
    "os",
    "os.path",
    "ossaudiodev",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pipes",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posix",
    "pprint",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtplib",
    "sndhdr",
    "socket",
    "socketserver",
    "spwd",
    "sqlite3",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sunau",
    "symtable",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "telnetlib",
    "tempfile",
    "termios",
    "test",
    "textwrap",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "tkinter.colorchooser",
    "tkinter.dnd",
    "tkinter.font",
    "tkinter.messagebox",
    "tkinter.scrolledtext",
    "tkinter.tix",
    "tkinter.ttk",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "unittest.mock",
    "urllib",
    "urllib.error",
    "urllib.parse",
    "urllib.request",
    "urllib.robotparser",
    "uu",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "winreg",
    "winsound",
    "wsgiref",
    "xdrlib",
    "xml.dom",
    "xml.dom.minidom",
    "xml.dom.pulldom",
    "xml.sax",
    "xml.sax.handler",
    "xmlrpc",
    "xmlrpc.client",
    "xmlrpc.server",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

/// The modules whose page has another name.
const OTHER_PAGE: &[(&str, &str)] = &[
    ("email.message.Message", "email.compat32-message"),
    ("tkinter.simpledialog", "dialog"),
    ("xml.etree.ElementTree", "xml.etree.elementtree"),
    ("xml.parsers.expat", "pyexpat"),
    ("xml.sax.saxutils", "xml.sax.utils"),
    ("xml.sax.xmlreader", "xml.sax.reader"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_in_hovers() {
        let ty = Markup::markdown(
            "```xml\n<module 'contextlib'>\n```\n---\nUtilities for with-statement contexts.",
        );
        assert_eq!(hovered_module(&ty).as_deref(), Some("contextlib"));
        let pyright = Markup::markdown("```python\n(module) os.path\n```\n---\nPaths.");
        assert_eq!(hovered_module(&pyright).as_deref(), Some("os.path"));
        let function = Markup::markdown("```python\ndef dumps(obj) -> str\n```");
        assert_eq!(hovered_module(&function), None);
        assert_eq!(hovered_module(&Markup::markdown("just text")), None);
        assert_eq!(
            hovered_module(&Markup::plain("(module) json")).as_deref(),
            Some("json")
        );
        assert_eq!(hovered_module(&Markup::plain("<module 'a b'>")), None);
    }

    #[test]
    fn standard_library_files() {
        let p = |s: &str| PathBuf::from(s);
        assert!(is_stdlib(&p(
            "/home/u/.cache/ty/vendored/typeshed/b932/stdlib/contextlib.pyi"
        )));
        assert!(is_stdlib(&p(
            "/x/basedpyright/dist/typeshed-fallback/stdlib/json/__init__.pyi"
        )));
        assert!(is_stdlib(&p(
            "/home/u/.local/share/uv/python/cpython-3.13/lib/python3.13/contextlib.py"
        )));
        assert!(!is_stdlib(&p(
            "/p/.venv/lib/python3.13/site-packages/requests/api.py"
        )));
        assert!(!is_stdlib(&p(
            "/x/typeshed-fallback/stubs/requests/requests/api.pyi"
        )));
        assert!(!is_stdlib(&p("/p/src/pkg/mod.py")));
    }

    #[test]
    fn links_to_docs_python_org() {
        assert_eq!(
            stdlib_url("contextlib", Some("3.13.1")),
            "https://docs.python.org/3.13/library/contextlib.html"
        );
        assert_eq!(
            stdlib_url("xml.parsers.expat", Some("3.14")),
            "https://docs.python.org/3.14/library/pyexpat.html"
        );
        // A module without a page of its own: the library index.
        assert_eq!(
            stdlib_url("_collections_abc", Some("3.12.0")),
            "https://docs.python.org/3.12/library/"
        );
        assert_eq!(
            stdlib_url("os.path", None),
            "https://docs.python.org/library/os.path.html"
        );
        assert_eq!(
            stdlib_url("os", Some("2.5.1")),
            "https://docs.python.org/2.5/lib/"
        );
        assert_eq!(major_minor("3.14.0rc1").as_deref(), Some("3.14"));
        assert_eq!(major_minor("x"), None);
    }

    #[test]
    fn module_documentation() {
        let stub = PathBuf::from("/c/typeshed/stdlib/contextlib.pyi");
        let doc = module_doc(&stub, "contextlib".into(), Some(Some("3.13.1".into())));
        assert_eq!(doc.kind, "Module");
        assert_eq!(doc.file_type, "PythonStub");
        assert_eq!(
            doc.external_url.as_deref(),
            Some("https://docs.python.org/3.13/library/contextlib.html")
        );
        // Without an interpreter, no link.
        assert_eq!(
            module_doc(&stub, "contextlib".into(), None).external_url,
            None
        );
        let package = PathBuf::from("/p/demo_pkg/__init__.py");
        let doc = module_doc(&package, "demo_pkg".into(), Some(Some("3.13".into())));
        assert_eq!(doc.kind, "Package");
        assert_eq!(doc.file_type, "Python");
        assert_eq!(doc.external_url, None);
    }
}
