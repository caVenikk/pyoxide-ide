//! A theme file that names roles this build does not know (saved by a newer
//! build) loads without them; known keys keep their values (issue #274).

use std::process::Command;

use themes::{Catalog, Registry, Rgba, TextRole, ThemeFile, UiRole, builtin};

const FILE: &str = r##"
[theme]
id = "from-the-future"
name = "From the future"
extends = "pyoxide-dark"

[ui]
accent = "#112233"
future-ui-role = "#445566"
another-new-role = "#778899"

[syntax]
keyword = { fg = "#aa0000", bold = true }
future-token = { fg = "#00aa00", italic = true, a-new-attribute = true }
"##;

const SKIPPED: [&str; 3] = [
    "ui.another-new-role",
    "ui.future-ui-role",
    "syntax.future-token",
];

#[test]
fn a_theme_with_unknown_roles_loads_and_keeps_the_known_ones() {
    let mut registry = Registry::with_builtin();
    registry.insert_toml(FILE, "future.toml").unwrap();
    let theme = registry.resolve("from-the-future").unwrap();
    let base = builtin::dark();
    assert_eq!(theme.ui[UiRole::Accent], Rgba::rgb(0x11, 0x22, 0x33));
    assert_eq!(
        theme.text[TextRole::Keyword].fg,
        Some(Rgba::rgb(0xaa, 0, 0))
    );
    assert!(theme.text[TextRole::Keyword].is_bold());
    assert_eq!(theme.ui[UiRole::Surface], base.ui[UiRole::Surface]);
    assert_eq!(theme.text[TextRole::String], base.text[TextRole::String]);
}

#[test]
fn the_skipped_keys_come_back_in_one_list() {
    let (_, skipped) = ThemeFile::parse_toml(FILE, "future.toml").unwrap();
    assert_eq!(skipped, SKIPPED);
    let warning = themes::skipped_warning("future.toml", &skipped);
    assert_eq!(warning.lines().count(), 1, "{warning}");
    for key in SKIPPED {
        assert!(warning.contains(key), "{key} is not in: {warning}");
    }
}

#[test]
fn a_theme_that_knows_every_key_has_nothing_to_report() {
    for file in builtin::files() {
        let (_, skipped) = ThemeFile::parse_toml(&file.to_toml(), "built-in").unwrap();
        assert!(skipped.is_empty(), "{}: {skipped:?}", file.theme.id);
    }
}

#[test]
fn the_catalog_offers_the_theme_and_reports_it_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("future.toml"), FILE).unwrap();
    let plain = FILE
        .replace("from-the-future", "plain")
        .replace("future-ui-role", "success")
        .replace("another-new-role = \"#778899\"\n", "")
        .replace("future-token", "comment")
        .replace(", a-new-attribute = true", "");
    std::fs::write(dir.path().join("plain.toml"), plain).unwrap();

    let (catalog, problems) = Catalog::load(dir.path());

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("future.toml"), "{problems:?}");
    for key in SKIPPED {
        assert!(problems[0].contains(key), "{key} is not in: {problems:?}");
    }
    assert!(catalog.is_theme("from-the-future"));
    assert!(catalog.is_theme("plain"));
    let theme = catalog.resolve("from-the-future").unwrap();
    assert_eq!(theme.ui[UiRole::Accent], Rgba::rgb(0x11, 0x22, 0x33));
    assert_eq!(
        theme.text[TextRole::Keyword].fg,
        Some(Rgba::rgb(0xaa, 0, 0))
    );
}

#[test]
fn other_errors_of_a_file_still_leave_it_out() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("bad-colour.toml"),
        FILE.replace("#112233", "not-a-colour"),
    )
    .unwrap();
    std::fs::write(dir.path().join("broken.toml"), "[ui\naccent = ").unwrap();

    let (catalog, problems) = Catalog::load(dir.path());

    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(!catalog.is_theme("from-the-future"));
    assert!(problems.iter().any(|p| p.contains("not-a-colour")));
}

const CHILD: &str = "THEMES_TEST_UNKNOWN_ROLES_CHILD";

/// Runs in a child process of `the_log_gets_one_warning_per_file`; on its
/// own it does nothing.
#[test]
fn log_child() {
    if std::env::var_os(CHILD).is_some() {
        ThemeFile::from_toml(FILE, "future.toml").unwrap();
    }
}

#[test]
fn the_log_gets_one_warning_per_file() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "unknown_roles::log_child", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("1 passed"),
        "the child did not run: {stdout}"
    );
    let warnings: Vec<&str> = stderr.lines().filter(|l| l.contains("skipped")).collect();
    assert_eq!(warnings.len(), 1, "{stderr}");
    assert!(warnings[0].starts_with("themes: future.toml: "), "{stderr}");
    for key in SKIPPED {
        assert!(warnings[0].contains(key), "{key} is not in: {stderr}");
    }
}

const NEWER: &str = r##"
[theme]
id = "newer"
name = "Newer"
extends = "pyoxide-dark"
a-new-header-field = "x"

[ui]
accent = "#112233"

[syntax]
keyword = { fg = "#aa0000", bold = true, glow = "#00ff00", sparkle = 3 }
string = { fg = "#00aa00" }
"##;

#[test]
fn unknown_attributes_and_header_fields_are_skipped_and_listed() {
    let (file, skipped) = ThemeFile::parse_toml(NEWER, "newer.toml").unwrap();
    assert_eq!(
        skipped,
        [
            "theme.a-new-header-field",
            "syntax.keyword.glow",
            "syntax.keyword.sparkle"
        ]
    );
    assert_eq!(file.theme.id, "newer");
    assert_eq!(file.theme.extends.as_deref(), Some("pyoxide-dark"));
    let mut registry = Registry::with_builtin();
    registry.insert(file).unwrap();
    let theme = registry.resolve("newer").unwrap();
    assert_eq!(theme.ui[UiRole::Accent], Rgba::rgb(0x11, 0x22, 0x33));
    assert_eq!(
        theme.text[TextRole::Keyword].fg,
        Some(Rgba::rgb(0xaa, 0, 0))
    );
    assert!(theme.text[TextRole::Keyword].is_bold());
    assert_eq!(theme.text[TextRole::String].fg, Some(Rgba::rgb(0, 0xaa, 0)));
    let warning = themes::skipped_warning("newer.toml", &skipped);
    assert_eq!(warning.lines().count(), 1, "{warning}");
}

#[test]
fn a_bad_value_of_a_known_attribute_or_header_field_is_still_an_error() {
    for text in [
        NEWER.replace("bold = true", "bold = \"loud\""),
        NEWER.replace("\"#aa0000\"", "\"red-ish\""),
        NEWER.replace("extends = \"pyoxide-dark\"", "extends = 3"),
        NEWER.replace("name = \"Newer\"", "kind = \"purple\""),
    ] {
        assert!(ThemeFile::parse_toml(&text, "bad.toml").is_err(), "{text}");
    }
}
