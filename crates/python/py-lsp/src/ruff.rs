//! Ruff's adjustments (ADR 0010), as the reference's
//! `RuffLspClient.customizeQuickFixes` makes them:
//!
//! - rule names instead of codes: a diagnostic's code (`F401` becomes
//!   `unused-import`, so it reads `Ruff(unused-import)`) and the quick fix
//!   titles (`Ruff (unused-import): Remove unused import: `os``), once
//!   the names are known (`ruff rule --output-format json --all`, run when
//!   the executable is found);
//! - ruff's own "Disable for this line" fix moves under the other fixes of
//!   its problem, when there are any;
//! - every fix of a problem gets the sub-actions, in the order the
//!   reference shows them: "Disable for this file" (`# ruff: noqa: CODE`
//!   on line 1, extending one that is there), "Disable for this line"
//!   (when it moved there), "Disable for this project" (`CODE` added to
//!   `ignore` in `[tool.ruff.lint]` of `pyproject.toml` or `[lint]` of
//!   `ruff.toml` / `.ruff.toml` in the file's folder or the project root;
//!   a `ruff.toml` created next to the file when there is none).
//!
//! One difference to the reference: a file that starts with a shebang
//! (`#!`) gets the comment on its second line, so the shebang keeps
//! working.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use code_intel::{
    CodeAction, CodeActionKind, Diagnostic, DocumentId, FileEdits, Point, ServerId, TextEdit,
    WorkspaceEdit,
};
use lsp_client::ServerCustomizer;
use process::CommandLine;
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::lookup::Executable;

/// The title of the sub-action that disables a rule in the file.
pub const DISABLE_FOR_FILE: &str = "Disable for this file";
/// The title ruff's own fix gets under the other fixes.
pub const DISABLE_FOR_LINE: &str = "Disable for this line";
/// The title of the sub-action that disables a rule in the project.
pub const DISABLE_FOR_PROJECT: &str = "Disable for this project";

/// What ruff's own "Disable for this line" fix is called (after the
/// `Ruff (CODE)` prefix).
const RUFF_DISABLE_LINE: &str = ": Disable for this line";
/// The id of ruff's server.
const RUFF: &str = "ruff";

/// Ruff's rules by code and by name.
#[derive(Debug, Default)]
struct Rules {
    names: HashMap<String, String>,
    codes: HashMap<String, String>,
}

/// Ruff's [`ServerCustomizer`]; it learns the rule names once the
/// executable is known.
#[derive(Debug)]
pub struct RuffCustomizer {
    /// The project root, where "Disable for this project" looks for ruff's
    /// configuration after the file's folder.
    root: PathBuf,
    rules: Mutex<Rules>,
}

impl RuffCustomizer {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            rules: Mutex::default(),
        }
    }

    /// Sets the rule names, code to name (from [`rules_command`]'s output,
    /// [`parse_rules`]).
    pub fn set_rules(&self, names: HashMap<String, String>) {
        let codes = names
            .iter()
            .map(|(code, name)| (name.clone(), code.clone()))
            .collect();
        *self.rules.lock().unwrap_or_else(PoisonError::into_inner) = Rules { names, codes };
    }

    /// The rule names are known.
    pub fn has_rules(&self) -> bool {
        !self
            .rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .names
            .is_empty()
    }

    /// The name of a rule code.
    pub fn rule_name(&self, code: &str) -> Option<String> {
        self.rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .names
            .get(code)
            .cloned()
    }

    /// The code of a rule, from its code or its name (a diagnostic's code
    /// after [`ServerCustomizer::diagnostic`] renamed it).
    pub fn rule_code(&self, code_or_name: &str) -> String {
        self.rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .codes
            .get(code_or_name)
            .cloned()
            .unwrap_or_else(|| code_or_name.to_owned())
    }

    /// A title with the rule name instead of the code: `Ruff (F401): ...`
    /// becomes `Ruff (unused-import): ...`.
    pub fn rename_title(&self, title: &str) -> String {
        let Some(start) = title.find("Ruff (") else {
            return title.to_owned();
        };
        let code_start = start + "Ruff (".len();
        let Some(len) = title[code_start..].find(')') else {
            return title.to_owned();
        };
        let code = &title[code_start..code_start + len];
        match self.rule_name(code) {
            Some(name) => format!(
                "{}{name}{}",
                &title[..code_start],
                &title[code_start + len..]
            ),
            None => title.to_owned(),
        }
    }

    /// Ruff's code actions as the reference shows them (see the module
    /// documentation). `text` is the document's text, for "Disable for
    /// this file"; without it that action has no edit.
    pub fn customize_actions(
        &self,
        doc: &DocumentId,
        text: Option<&str>,
        actions: &mut Vec<CodeAction>,
    ) {
        // The quick fixes of each problem, in order.
        let mut groups: Vec<(Diagnostic, Vec<usize>)> = Vec::new();
        for (i, action) in actions.iter().enumerate() {
            if !action.kind.is_quick_fix() {
                continue;
            }
            let Some(diagnostic) = action.diagnostics.first() else {
                continue;
            };
            match groups.iter_mut().find(|(d, _)| same_problem(d, diagnostic)) {
                Some((_, members)) => members.push(i),
                None => groups.push((diagnostic.clone(), vec![i])),
            }
        }

        let mut nested_away = Vec::new();
        for (diagnostic, members) in &groups {
            let Some(code) = diagnostic.code.as_deref().map(|c| self.rule_code(c)) else {
                continue;
            };
            let disable_line = members
                .iter()
                .copied()
                .find(|&i| actions[i].title.contains(RUFF_DISABLE_LINE));
            let nested = members.len() > 1 && disable_line.is_some();
            let line_action = disable_line.filter(|_| nested).map(|i| {
                let mut line = actions[i].clone();
                line.title = DISABLE_FOR_LINE.to_owned();
                line.children.clear();
                line
            });
            let file_action = self.disable_for_file_action(doc, text, diagnostic, &code);
            let project_action = self.disable_for_project_action(doc, diagnostic, &code);
            for &i in members {
                if nested && Some(i) == disable_line {
                    nested_away.push(i);
                    continue;
                }
                let mut children = vec![file_action.clone()];
                children.extend(line_action.clone());
                children.push(project_action.clone());
                actions[i].children = children;
            }
        }
        nested_away.sort_unstable();
        for i in nested_away.into_iter().rev() {
            actions.remove(i);
        }
        for action in actions.iter_mut() {
            action.title = self.rename_title(&action.title);
        }
    }

    fn disable_for_file_action(
        &self,
        doc: &DocumentId,
        text: Option<&str>,
        diagnostic: &Diagnostic,
        code: &str,
    ) -> CodeAction {
        let edit = text.map(|text| WorkspaceEdit {
            files: vec![FileEdits {
                path: doc.path().to_owned(),
                edits: disable_for_file(text, code).into_iter().collect(),
            }],
        });
        added_action(DISABLE_FOR_FILE, diagnostic, edit)
    }

    fn disable_for_project_action(
        &self,
        doc: &DocumentId,
        diagnostic: &Diagnostic,
        code: &str,
    ) -> CodeAction {
        let edit = disable_for_project(doc.path(), &self.root, code)
            .map(|file| WorkspaceEdit { files: vec![file] });
        added_action(DISABLE_FOR_PROJECT, diagnostic, edit)
    }
}

/// An action of the product's own for a problem of ruff's.
fn added_action(title: &str, diagnostic: &Diagnostic, edit: Option<WorkspaceEdit>) -> CodeAction {
    let mut action = CodeAction::new(title, CodeActionKind::QuickFix);
    action.diagnostics = vec![diagnostic.clone()];
    action.edit = Some(edit.unwrap_or_default());
    action.server = Some(ServerId::new(RUFF));
    action
}

/// Two diagnostics are the same problem: same place, rule and message
/// (the tokens may differ).
fn same_problem(a: &Diagnostic, b: &Diagnostic) -> bool {
    a.range == b.range && a.code == b.code && a.message == b.message
}

impl ServerCustomizer for RuffCustomizer {
    fn diagnostic(&self, diagnostic: &mut Diagnostic) {
        if let Some(name) = diagnostic.code.as_deref().and_then(|c| self.rule_name(c)) {
            diagnostic.code = Some(name);
        }
    }

    fn code_actions(
        &self,
        doc: &DocumentId,
        _: std::ops::Range<Point>,
        _: &code_intel::CodeActionContext,
        actions: &mut Vec<CodeAction>,
    ) {
        // Customizers do not get the document's text: the file on disk
        // stands in for it (auto-save keeps it close).
        let text = std::fs::read_to_string(doc.path()).ok();
        self.customize_actions(doc, text.as_deref(), actions);
    }
}

/// The command that lists ruff's rules as JSON (code, name, ...).
pub fn rules_command(executable: &Executable) -> CommandLine {
    executable.command_line(&["rule", "--output-format", "json", "--all"])
}

/// The most output of [`rules_command`] that is read (about 1.5 MB with
/// ruff 0.15).
const RULES_OUTPUT_LIMIT: usize = 32 * 1024 * 1024;

/// Ruff's rule names, code to name (blocking: runs [`rules_command`] for
/// at most `timeout`); empty when it fails.
pub fn load_rules(
    executable: &Executable,
    timeout: std::time::Duration,
) -> HashMap<String, String> {
    match crate::lookup::run_for_output(&rules_command(executable), timeout, RULES_OUTPUT_LIMIT) {
        Ok((stdout, _, true)) => parse_rules(&stdout),
        _ => HashMap::new(),
    }
}

/// Rule code to name from the output of [`rules_command`]: an array of
/// objects with `code` and `name`.
pub fn parse_rules(json: &str) -> HashMap<String, String> {
    let Ok(serde_json::Value::Array(rules)) = serde_json::from_str(json) else {
        return HashMap::new();
    };
    rules
        .iter()
        .filter_map(|rule| {
            let code = rule.get("code")?.as_str()?;
            let name = rule.get("name")?.as_str()?;
            Some((code.to_owned(), name.to_owned()))
        })
        .collect()
}

// ---- Disable for this file ----------------------------------------------------

/// The edit that disables `code` in the whole file: `# ruff: noqa: CODE`
/// inserted as the first line, or `CODE` added to such a comment there.
/// `None` when the comment has it already.
pub fn disable_for_file(text: &str, code: &str) -> Option<TextEdit> {
    let (line_index, line_start) = if text.starts_with("#!") {
        (1, text.find('\n').map_or(text.len(), |i| i + 1))
    } else {
        (0, 0)
    };
    let rest = &text[line_start..];
    let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
    let re = regex::Regex::new(r"#\s*ruff:\s*noqa:\s*(.*?)(#|$)").expect("the pattern is valid");
    let Some(found) = re.captures(line) else {
        let at = Point::new(line_index, 0);
        let newline = if line_start == text.len() && line_index > 0 && !text.ends_with('\n') {
            // A shebang without a line break after it.
            return Some(TextEdit::insert(
                Point::new(0, text.chars().count()),
                format!("\n# ruff: noqa: {code}"),
            ));
        } else {
            "\n"
        };
        return Some(TextEdit::insert(
            at,
            format!("# ruff: noqa: {code}{newline}"),
        ));
    };
    let codes_match = found.get(1).expect("the group is there");
    let mut codes: Vec<&str> = codes_match
        .as_str()
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .collect();
    if codes.contains(&code) {
        return None;
    }
    codes.push(code);
    let start = found.get(0).expect("the match is there").start();
    let end = codes_match.start() + codes_match.as_str().trim_end().len();
    let column = |byte: usize| line[..byte].chars().count();
    Some(TextEdit::new(
        Point::new(line_index, column(start))..Point::new(line_index, column(end)),
        format!("# ruff: noqa: {}", codes.join(", ")),
    ))
}

// ---- Disable for this project -------------------------------------------------

/// The configuration file a rule is disabled in: `.ruff.toml`, `ruff.toml`
/// or `pyproject.toml` in the file's folder, then in the project root.
pub fn config_file(file: &Path, root: &Path) -> Option<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(parent) = file.parent() {
        dirs.push(parent);
    }
    if !dirs.contains(&root) {
        dirs.push(root);
    }
    dirs.into_iter().find_map(|dir| {
        [".ruff.toml", "ruff.toml", "pyproject.toml"]
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    })
}

/// The edits that disable `code` in the project ([`config_file`]'s
/// `ignore`, or a new `ruff.toml` next to the file). `None` when the rule
/// is ignored there already or the file is not TOML.
pub fn disable_for_project(file: &Path, root: &Path, code: &str) -> Option<FileEdits> {
    let Some(config) = config_file(file, root) else {
        return Some(FileEdits {
            path: file.parent()?.join("ruff.toml"),
            edits: vec![TextEdit::insert(
                Point::new(0, 0),
                format!("[lint]\nignore = [\"{code}\"]\n"),
            )],
        });
    };
    let text = std::fs::read_to_string(&config).ok()?;
    let section = if config.file_name().is_some_and(|n| n == "pyproject.toml") {
        "tool.ruff.lint"
    } else {
        "lint"
    };
    let new = add_to_ignore(&text, section, code)?;
    Some(FileEdits {
        path: config,
        edits: vec![minimal_edit(&text, &new)],
    })
}

/// `text` (TOML) with `code` added to `ignore` of the table `section`
/// (dotted), the table created when missing. `None` when `code` is there
/// already or the text is not TOML. As the reference writes it: an array
/// of one item on one line, of more one item per line.
pub fn add_to_ignore(text: &str, section: &str, code: &str) -> Option<String> {
    let mut doc: DocumentMut = text.parse().ok()?;
    let parts: Vec<&str> = section.split('.').collect();
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for (i, part) in parts.iter().enumerate() {
        if table.get(part).is_none() {
            let mut new = Table::new();
            new.set_implicit(i + 1 < parts.len());
            table.insert(part, Item::Table(new));
        }
        table = table.get_mut(part)?.as_table_like_mut()?;
    }
    let (mut codes, decor) = match table.get("ignore") {
        Some(Item::Value(Value::Array(array))) => (
            array
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect::<Vec<_>>(),
            Some(array.decor().clone()),
        ),
        Some(Item::Value(Value::String(s))) => (vec![s.value().clone()], Some(s.decor().clone())),
        Some(Item::Value(other)) => (Vec::new(), Some(other.decor().clone())),
        _ => (Vec::new(), None),
    };
    if codes.iter().any(|c| c == code) {
        return None;
    }
    codes.push(code.to_owned());
    let mut array = Array::new();
    for c in &codes {
        array.push(c.as_str());
    }
    if codes.len() > 1 {
        for v in array.iter_mut() {
            v.decor_mut().set_prefix("\n    ");
            v.decor_mut().set_suffix("");
        }
        array.set_trailing("\n");
        array.set_trailing_comma(true);
    }
    if let Some(decor) = decor {
        *array.decor_mut() = decor;
    }
    table.insert("ignore", Item::Value(Value::Array(array)));
    let mut out = doc.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

/// One edit that turns `old` into `new`: the part between their common
/// beginning and end.
fn minimal_edit(old: &str, new: &str) -> TextEdit {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    TextEdit::new(
        point_at(old, prefix)..point_at(old, old.len() - suffix),
        &new[prefix..new.len() - suffix],
    )
}

/// The position of a byte offset of `text`.
fn point_at(text: &str, byte: usize) -> Point {
    let before = &text[..byte];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    Point::new(
        before.matches('\n').count(),
        before[line_start..].chars().count(),
    )
}

#[cfg(test)]
mod tests {
    use code_intel::Severity;

    use super::*;

    /// Applies non-overlapping edits of one file to `text`.
    fn apply(text: &str, edits: &[TextEdit]) -> String {
        let offset = |p: Point| {
            let mut line_start = 0;
            for _ in 0..p.line {
                line_start += text[line_start..].find('\n').unwrap() + 1;
            }
            line_start
                + text[line_start..]
                    .chars()
                    .take(p.column)
                    .map(char::len_utf8)
                    .sum::<usize>()
        };
        let mut edits: Vec<&TextEdit> = edits.iter().collect();
        edits.sort_by_key(|e| std::cmp::Reverse((e.range.start.line, e.range.start.column)));
        let mut out = text.to_owned();
        for e in edits {
            out.replace_range(offset(e.range.start)..offset(e.range.end), &e.text);
        }
        out
    }

    fn ruff_in(root: &Path) -> RuffCustomizer {
        let ruff = RuffCustomizer::new(root);
        ruff.set_rules(HashMap::from([
            ("F401".into(), "unused-import".into()),
            ("E711".into(), "none-comparison".into()),
        ]));
        ruff
    }

    #[test]
    fn rule_names() {
        let ruff = RuffCustomizer::new("/p");
        assert_eq!(ruff.rule_name("F401"), None);
        assert!(!ruff.has_rules());
        assert_eq!(
            ruff.rename_title("Ruff (F401): Remove unused import"),
            "Ruff (F401): Remove unused import"
        );
        ruff.set_rules(HashMap::from([("F401".into(), "unused-import".into())]));
        assert!(ruff.has_rules());
        assert_eq!(ruff.rule_name("F401").as_deref(), Some("unused-import"));
        assert_eq!(ruff.rule_code("unused-import"), "F401");
        assert_eq!(ruff.rule_code("F401"), "F401");
        assert_eq!(
            ruff.rename_title("Ruff (F401): Remove unused import: `os`"),
            "Ruff (unused-import): Remove unused import: `os`"
        );
        assert_eq!(
            ruff.rename_title("Ruff: Fix all auto-fixable problems"),
            "Ruff: Fix all auto-fixable problems"
        );
        let command = rules_command(&Executable::Path("ruff".into()));
        assert_eq!(command.display(), "ruff rule --output-format json --all");

        let mut d = Diagnostic::new(
            Point::new(0, 7)..Point::new(0, 9),
            Severity::Warning,
            "`os` imported but unused",
        );
        d.source = Some("Ruff".into());
        d.code = Some("F401".into());
        ruff.diagnostic(&mut d);
        assert_eq!(d.summary(), "`os` imported but unused  Ruff(unused-import)");
        d.code = Some("E999".into());
        ruff.diagnostic(&mut d);
        assert_eq!(d.code.as_deref(), Some("E999"));
    }

    #[test]
    fn rules_from_ruff() {
        let json = r#"[
          {"name": "unused-import", "code": "F401", "linter": "Pyflakes", "preview": false},
          {"name": "none-comparison", "code": "E711", "linter": "pycodestyle"},
          {"code": "X1"}
        ]"#;
        let rules = parse_rules(json);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules["E711"], "none-comparison");
        assert!(parse_rules("not json").is_empty());
        assert!(parse_rules("{}").is_empty());
    }

    #[test]
    fn noqa_for_the_file() {
        let insert = |text: &str| apply(text, &[disable_for_file(text, "F401").unwrap()]);
        assert_eq!(insert("import os\n"), "# ruff: noqa: F401\nimport os\n");
        assert_eq!(insert(""), "# ruff: noqa: F401\n");
        assert_eq!(
            insert("# ruff: noqa: E711\nimport os\n"),
            "# ruff: noqa: E711, F401\nimport os\n"
        );
        assert_eq!(
            insert("#ruff:noqa:E711,E712 # legacy\nx\n"),
            "# ruff: noqa: E711, E712, F401 # legacy\nx\n"
        );
        assert_eq!(disable_for_file("# ruff: noqa: F401, E711\n", "F401"), None);
        assert_eq!(disable_for_file("# ruff: noqa: E711,F401\n", "F401"), None);
        // Only the first line counts.
        assert_eq!(
            insert("import os\n# ruff: noqa: E711\n"),
            "# ruff: noqa: F401\nimport os\n# ruff: noqa: E711\n"
        );
        // A shebang stays first.
        assert_eq!(
            insert("#!/usr/bin/env python\nimport os\n"),
            "#!/usr/bin/env python\n# ruff: noqa: F401\nimport os\n"
        );
        assert_eq!(
            insert("#!/usr/bin/env python\n# ruff: noqa: E711\n"),
            "#!/usr/bin/env python\n# ruff: noqa: E711, F401\n"
        );
        assert_eq!(
            insert("#!/usr/bin/env python"),
            "#!/usr/bin/env python\n# ruff: noqa: F401"
        );
        // Columns count characters.
        let text = "# ruff: noqa: E711 # é\n";
        let edit = disable_for_file(text, "F401").unwrap();
        assert_eq!(edit.range, Point::new(0, 0)..Point::new(0, 18));
    }

    #[test]
    fn ignore_in_pyproject() {
        let add = |text: &str| add_to_ignore(text, "tool.ruff.lint", "F401");
        assert_eq!(
            add("[project]\nname = \"demo\"\n").unwrap(),
            "[project]\nname = \"demo\"\n\n[tool.ruff.lint]\nignore = [\"F401\"]\n"
        );
        assert_eq!(
            add("[project]\nname = \"demo\"\n\n[tool.ruff]\nline-length = 100\n\n[tool.other]\nx = 1\n")
                .unwrap(),
            "[project]\nname = \"demo\"\n\n[tool.ruff]\nline-length = 100\n\n[tool.ruff.lint]\nignore = [\"F401\"]\n\n[tool.other]\nx = 1\n"
        );
        assert_eq!(
            add("[tool.ruff.lint]\nselect = [\"E\", \"F\"]\nignore = [\"E501\"]  # long lines\n")
                .unwrap(),
            "[tool.ruff.lint]\nselect = [\"E\", \"F\"]\nignore = [\n    \"E501\",\n    \"F401\",\n]  # long lines\n"
        );
        assert_eq!(
            add("[tool.ruff.lint]\nselect = [\"E\"]\n").unwrap(),
            "[tool.ruff.lint]\nselect = [\"E\"]\nignore = [\"F401\"]\n"
        );
        assert_eq!(
            add("[tool.ruff.lint]\nignore = \"E501\"\n").unwrap(),
            "[tool.ruff.lint]\nignore = [\n    \"E501\",\n    \"F401\",\n]\n"
        );
        assert_eq!(add("[tool.ruff.lint]\nignore = [\"F401\"]\n"), None);
        assert_eq!(add("not = toml = at all"), None);
        assert_eq!(
            add_to_ignore("line-length = 100\n", "lint", "F401").unwrap(),
            "line-length = 100\n\n[lint]\nignore = [\"F401\"]\n"
        );
    }

    #[test]
    fn disable_for_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pkg = root.join("pkg");
        std::fs::create_dir_all(&pkg).unwrap();
        let file = pkg.join("app.py");

        // Nothing: a ruff.toml next to the file.
        let edits = disable_for_project(&file, root, "F401").unwrap();
        assert_eq!(edits.path, pkg.join("ruff.toml"));
        assert_eq!(apply("", &edits.edits), "[lint]\nignore = [\"F401\"]\n");

        // pyproject.toml of the project.
        let pyproject = "[project]\nname = \"demo\"\n";
        std::fs::write(root.join("pyproject.toml"), pyproject).unwrap();
        let edits = disable_for_project(&file, root, "F401").unwrap();
        assert_eq!(edits.path, root.join("pyproject.toml"));
        assert_eq!(edits.edits.len(), 1);
        assert_eq!(edits.edits[0].range.start, Point::new(2, 0));
        assert_eq!(
            apply(pyproject, &edits.edits),
            "[project]\nname = \"demo\"\n\n[tool.ruff.lint]\nignore = [\"F401\"]\n"
        );

        // ruff.toml wins over pyproject.toml; the file's folder over the
        // root; .ruff.toml over ruff.toml.
        std::fs::write(root.join("ruff.toml"), "[lint]\nignore = [\"E501\"]\n").unwrap();
        assert_eq!(config_file(&file, root), Some(root.join("ruff.toml")));
        let edits = disable_for_project(&file, root, "F401").unwrap();
        assert_eq!(
            apply("[lint]\nignore = [\"E501\"]\n", &edits.edits),
            "[lint]\nignore = [\n    \"E501\",\n    \"F401\",\n]\n"
        );
        assert_eq!(disable_for_project(&file, root, "E501"), None);
        std::fs::write(root.join(".ruff.toml"), "").unwrap();
        assert_eq!(config_file(&file, root), Some(root.join(".ruff.toml")));
        std::fs::write(pkg.join("pyproject.toml"), "").unwrap();
        assert_eq!(config_file(&file, root), Some(pkg.join("pyproject.toml")));
        let edits = disable_for_project(&file, root, "F401").unwrap();
        assert_eq!(
            apply("", &edits.edits),
            "\n[tool.ruff.lint]\nignore = [\"F401\"]\n".trim_start()
        );
    }

    /// Ruff's actions for an unused import, as its server sends them.
    fn server_actions(diagnostic: &Diagnostic) -> Vec<CodeAction> {
        let fix = |title: &str, kind: CodeActionKind| {
            let mut a = CodeAction::new(title, kind);
            a.server = Some(ServerId::new("ruff"));
            a
        };
        let mut remove = fix(
            "Ruff (F401): Remove unused import: `os`",
            CodeActionKind::QuickFix,
        );
        remove.preferred = true;
        remove.diagnostics = vec![diagnostic.clone()];
        let mut line = fix(
            "Ruff (F401): Disable for this line",
            CodeActionKind::QuickFix,
        );
        line.diagnostics = vec![diagnostic.clone()];
        vec![
            remove,
            line,
            fix(
                "Ruff: Fix all auto-fixable problems",
                CodeActionKind::Source("fixAll.ruff".into()),
            ),
            fix(
                "Ruff: Organize imports",
                CodeActionKind::Source("organizeImports.ruff".into()),
            ),
        ]
    }

    fn unused_os(code: &str) -> Diagnostic {
        let mut d = Diagnostic::new(
            Point::new(0, 7)..Point::new(0, 9),
            Severity::Warning,
            "`os` imported but unused",
        );
        d.code = Some(code.into());
        d
    }

    #[test]
    fn quick_fixes_as_the_reference_shows_them() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("app.py");
        let text = "import os\n";
        let doc = DocumentId::new(&file);
        let ruff = ruff_in(dir.path());
        // The diagnostic as renamed by `diagnostic`, or not yet.
        for code in ["unused-import", "F401"] {
            let mut actions = server_actions(&unused_os(code));
            ruff.customize_actions(&doc, Some(text), &mut actions);
            let titles: Vec<&str> = actions.iter().map(|a| a.title.as_str()).collect();
            assert_eq!(
                titles,
                [
                    "Ruff (unused-import): Remove unused import: `os`",
                    "Ruff: Fix all auto-fixable problems",
                    "Ruff: Organize imports",
                ]
            );
            let subs: Vec<&str> = actions[0]
                .children
                .iter()
                .map(|a| a.title.as_str())
                .collect();
            assert_eq!(
                subs,
                [DISABLE_FOR_FILE, DISABLE_FOR_LINE, DISABLE_FOR_PROJECT]
            );
            assert!(actions[1].children.is_empty());

            let file_edit = actions[0].children[0].edit.as_ref().unwrap();
            assert_eq!(
                apply(
                    text,
                    &file_edit.edits_for(&file).cloned().collect::<Vec<_>>()
                ),
                "# ruff: noqa: F401\nimport os\n"
            );
            let project = actions[0].children[2].edit.as_ref().unwrap();
            assert_eq!(project.files[0].path, dir.path().join("ruff.toml"));
            assert_eq!(
                project.files[0].edits[0].text,
                "[lint]\nignore = [\"F401\"]\n"
            );
            assert!(
                actions[0]
                    .children
                    .iter()
                    .all(|c| c.server == Some(ServerId::new("ruff")))
            );
        }
    }

    #[test]
    fn a_lone_disable_for_this_line_stays() {
        let doc = DocumentId::new("/p/app.py");
        let ruff = ruff_in(Path::new("/p"));
        let diagnostic = unused_os("F401");
        let mut actions = server_actions(&diagnostic);
        actions.remove(0);
        ruff.customize_actions(&doc, None, &mut actions);
        assert_eq!(
            actions[0].title,
            "Ruff (unused-import): Disable for this line"
        );
        let subs: Vec<&str> = actions[0]
            .children
            .iter()
            .map(|a| a.title.as_str())
            .collect();
        assert_eq!(subs, [DISABLE_FOR_FILE, DISABLE_FOR_PROJECT]);
        // Without the text the file action changes nothing.
        assert!(actions[0].children[0].edit.as_ref().unwrap().is_empty());
        assert_eq!(actions.len(), 3);
    }

    #[test]
    fn edits_and_positions() {
        let edit = minimal_edit("a\nbc\n", "a\nbXc\n");
        assert_eq!(edit.range, Point::new(1, 1)..Point::new(1, 1));
        assert_eq!(edit.text, "X");
        let edit = minimal_edit("é\nx", "é\ny");
        assert_eq!(edit.range, Point::new(1, 0)..Point::new(1, 1));
        assert_eq!(minimal_edit("same", "same").text, "");
        assert_eq!(point_at("ab\ncd", 4), Point::new(1, 1));
    }
}
