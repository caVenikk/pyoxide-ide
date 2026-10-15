//! Paths of run configurations. They are stored relative to the project
//! root with `/` separators (ADR 0012). The reference IDE writes them with
//! path variables (`PathMacroUtil`): `$PROJECT_DIR$` (the folder that holds
//! `.idea`), `$PROJECT_DIR$/..` (its parent), `$MODULE_DIR$` (the module's
//! folder, the project folder for a module file in `.idea`) and
//! `$USER_HOME$`, and user path variables as `$NAME$`. On import the
//! project and module variables become relative paths; `$USER_HOME$` and
//! other variables are kept as written and flagged in the editor.

use std::path::{Component, Path, PathBuf};

pub const PROJECT_DIR: &str = "$PROJECT_DIR$";
pub const MODULE_DIR: &str = "$MODULE_DIR$";
pub const USER_HOME: &str = "$USER_HOME$";

/// The stored form of the project folder itself.
pub const PROJECT: &str = ".";

/// A path as the reference wrote it, in the stored form.
pub fn from_reference(value: &str, root: &Path) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let value = value.replace('\\', "/");
    for var in [PROJECT_DIR, MODULE_DIR] {
        if let Some(rest) = strip_var(&value, var) {
            return normalize_relative(rest);
        }
    }
    if value.starts_with('$') {
        // `$USER_HOME$` and user path variables stay as written.
        return value;
    }
    if is_absolute(&value) {
        let path = lexical(Path::new(&value));
        let root = lexical(root);
        if let Ok(rest) = path.strip_prefix(&root) {
            let rest = to_slashes(rest);
            return if rest.is_empty() {
                PROJECT.to_owned()
            } else {
                rest
            };
        }
        return value;
    }
    normalize_relative(&value)
}

/// `$VAR$` at the start of `value`, followed by the end or `/`: the rest.
fn strip_var<'a>(value: &'a str, var: &str) -> Option<&'a str> {
    let rest = value.strip_prefix(var)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

/// A relative path with `.` and inner `..` resolved; leading `..` kept;
/// the project folder itself is [`PROJECT`].
fn normalize_relative(rest: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        PROJECT.to_owned()
    } else {
        parts.join("/")
    }
}

/// An absolute path on any OS: `/x`, `C:/x`, `C:\x`, `\\server\share`.
pub fn is_absolute(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || value.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'/' || bytes[2] == b'\\'))
}

/// A stored path as a path on this machine: relative paths are inside the
/// project, `$PROJECT_DIR$`, `$MODULE_DIR$` and `$USER_HOME$` are expanded,
/// other variables are left as they are (the file will not be found).
/// `None` for an empty path.
pub fn resolve(stored: &str, root: &Path) -> Option<PathBuf> {
    let stored = stored.trim();
    if stored.is_empty() {
        return None;
    }
    let (base, rest) = if let Some(rest) = strip_var(stored, PROJECT_DIR) {
        (root.to_owned(), rest)
    } else if let Some(rest) = strip_var(stored, MODULE_DIR) {
        (root.to_owned(), rest)
    } else if let Some(rest) = strip_var(stored, USER_HOME) {
        match home() {
            Some(home) => (home, rest),
            None => return Some(PathBuf::from(stored)),
        }
    } else if stored.starts_with('$') || is_absolute(stored) {
        return Some(PathBuf::from(stored));
    } else {
        (root.to_owned(), stored)
    };
    let mut out = base;
    for part in rest.split(['/', '\\']).filter(|p| !p.is_empty()) {
        out.push(part);
    }
    Some(lexical(&out))
}

/// Expands the path variables in a parameter string (`$PROJECT_DIR$`,
/// `$MODULE_DIR$`, `$USER_HOME$`); other macros (`$Prompt$`) stay as
/// they are.
pub fn expand_in_text(text: &str, root: &Path) -> String {
    let mut out = text.to_owned();
    let root = root.display().to_string();
    out = out.replace(PROJECT_DIR, &root).replace(MODULE_DIR, &root);
    if out.contains(USER_HOME)
        && let Some(home) = home()
    {
        out = out.replace(USER_HOME, &home.display().to_string());
    }
    out
}

/// The path variable a stored path starts with, when it is one the editor
/// flags: `$USER_HOME$` (machine-specific) or a variable Pyoxide does not
/// know.
pub fn flagged_variable(stored: &str) -> Option<&str> {
    let rest = stored.strip_prefix('$')?;
    let end = rest.find('$')?;
    let var = &stored[..end + 2];
    (var != PROJECT_DIR && var != MODULE_DIR).then_some(var)
}

/// The home folder.
fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn to_slashes(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// `path` with `.` and `..` resolved lexically.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_paths_become_relative() {
        let root = if cfg!(windows) {
            Path::new(r"C:\work\demo")
        } else {
            Path::new("/work/demo")
        };
        assert_eq!(
            from_reference("$PROJECT_DIR$/app/main.py", root),
            "app/main.py"
        );
        assert_eq!(from_reference("$PROJECT_DIR$", root), ".");
        assert_eq!(from_reference("$PROJECT_DIR$/", root), ".");
        assert_eq!(from_reference("$PROJECT_DIR$/..", root), "..");
        assert_eq!(
            from_reference("$PROJECT_DIR$/../shared/.env", root),
            "../shared/.env"
        );
        assert_eq!(from_reference("$PROJECT_DIR$/a/./b/../c", root), "a/c");
        assert_eq!(from_reference("$MODULE_DIR$/src", root), "src");
        assert_eq!(from_reference("$USER_HOME$/.env", root), "$USER_HOME$/.env");
        assert_eq!(from_reference("$MY_TOOLS$/x.py", root), "$MY_TOOLS$/x.py");
        assert_eq!(from_reference("$PROJECT_DIRX$/x", root), "$PROJECT_DIRX$/x");
        assert_eq!(from_reference("", root), "");
        assert_eq!(from_reference("relative\\dir", root), "relative/dir");
        if cfg!(windows) {
            assert_eq!(from_reference("C:/work/demo/app.py", root), "app.py");
            assert_eq!(
                from_reference("D:/elsewhere/app.py", root),
                "D:/elsewhere/app.py"
            );
        } else {
            assert_eq!(from_reference("/work/demo/app.py", root), "app.py");
            assert_eq!(from_reference("/work/demo", root), ".");
            assert_eq!(
                from_reference("/opt/tools/app.py", root),
                "/opt/tools/app.py"
            );
        }
    }

    #[test]
    fn stored_paths_resolve_on_this_machine() {
        let root = std::env::temp_dir().join("demo");
        assert_eq!(resolve("", &root), None);
        assert_eq!(resolve(".", &root), Some(root.clone()));
        assert_eq!(
            resolve("app/main.py", &root),
            Some(root.join("app").join("main.py"))
        );
        assert_eq!(
            resolve("../x", &root),
            Some(lexical(&root.join("..").join("x")))
        );
        assert_eq!(resolve("$PROJECT_DIR$/a", &root), Some(root.join("a")));
        assert_eq!(
            resolve("$UNKNOWN$/a", &root),
            Some(PathBuf::from("$UNKNOWN$/a"))
        );
        if let Some(home) = home() {
            assert_eq!(
                resolve("$USER_HOME$/.env", &root),
                Some(lexical(&home.join(".env")))
            );
        }
        let abs = if cfg!(windows) {
            r"C:\tools\x.py"
        } else {
            "/tools/x.py"
        };
        assert_eq!(resolve(abs, &root), Some(PathBuf::from(abs)));
    }

    #[test]
    fn variables_are_flagged_and_expanded_in_text() {
        assert_eq!(flagged_variable("$USER_HOME$/.env"), Some("$USER_HOME$"));
        assert_eq!(flagged_variable("$TOOLS$/x"), Some("$TOOLS$"));
        assert_eq!(flagged_variable("$PROJECT_DIR$/x"), None);
        assert_eq!(flagged_variable("app.py"), None);
        let root = Path::new("/w/p");
        assert_eq!(
            expand_in_text("--config $PROJECT_DIR$/c.yml $Prompt$", root),
            format!("--config {}/c.yml $Prompt$", root.display())
        );
    }
}
