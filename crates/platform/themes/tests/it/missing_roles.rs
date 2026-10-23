//! A stored theme written by an older version lacks the roles added since;
//! the theme it was made on supplies them and nothing is reported
//! (issue #326).

use themes::{Catalog, Rgba, ThemeFile, ThemeKind, UiRole, builtin};

/// Roles an older version did not have.
const LATER: [&str; 3] = [
    "selection-light",
    "tab-selected-inactive",
    "tab-underline-inactive",
];

/// The self-contained file an older version would have stored for `base`,
/// as `id`: all the colours of the base, but not the later roles.
fn old_file(base: ThemeKind, id: &str) -> String {
    let mut file = builtin::theme(base).to_file();
    file.theme.id = id.into();
    file.theme.name = id.into();
    // The accent is the user's own.
    file.ui.insert(
        UiRole::Accent,
        themes::ColorRef::Rgba(Rgba::rgb(0x12, 0x34, 0x56)),
    );
    let text = file.to_toml();
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !LATER.iter().any(|r| l.starts_with(&format!("{r} "))))
        .collect();
    assert_eq!(kept.len() + LATER.len(), text.lines().count(), "{text}");
    kept.join("\n")
}

#[test]
fn an_old_theme_loads_with_the_values_of_its_base_and_no_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("old.toml"),
        old_file(ThemeKind::Dark, "old"),
    )
    .unwrap();

    let (catalog, problems) = Catalog::load(dir.path());

    assert!(problems.is_empty(), "{problems:?}");
    assert!(catalog.is_theme("old"));
    let theme = catalog.resolve("old").unwrap();
    // The default theme of its kind supplies the roles; its own colours stay.
    let base = builtin::default_theme(ThemeKind::Dark);
    assert_eq!(
        theme.ui[UiRole::SelectionLight],
        base.ui[UiRole::SelectionLight]
    );
    assert_eq!(
        theme.ui[UiRole::TabSelectedInactive],
        base.ui[UiRole::TabSelectedInactive]
    );
    assert_eq!(
        theme.ui[UiRole::TabUnderlineInactive],
        base.ui[UiRole::TabUnderlineInactive]
    );
    assert_eq!(theme.ui[UiRole::Accent], Rgba::rgb(0x12, 0x34, 0x56));
    assert_eq!(
        theme.ui[UiRole::Surface],
        builtin::dark().ui[UiRole::Surface]
    );
}

#[test]
fn the_recorded_base_supplies_the_roles() {
    let text = old_file(ThemeKind::Dark, "old").replace(
        "id = \"old\"",
        &format!("id = \"old\"\nbase = \"{}\"", builtin::DARK_ID),
    );
    let file = ThemeFile::from_toml(&text, "old.toml").unwrap();
    assert_eq!(file.theme.base.as_deref(), Some(builtin::DARK_ID));
    let mut registry = themes::Registry::with_builtin();
    registry.insert(file).unwrap();
    let theme = registry.resolve("old").unwrap();
    let base = builtin::dark();
    assert_eq!(
        theme.ui[UiRole::SelectionLight],
        base.ui[UiRole::SelectionLight]
    );
    assert_ne!(
        theme.ui[UiRole::SelectionLight],
        builtin::default_theme(ThemeKind::Dark).ui[UiRole::SelectionLight]
    );
}

#[test]
fn a_light_theme_takes_the_light_default() {
    let mut registry = themes::Registry::with_builtin();
    registry
        .insert_toml(&old_file(ThemeKind::Light, "old-light"), "old.toml")
        .unwrap();
    let theme = registry.resolve("old-light").unwrap();
    assert_eq!(theme.kind, ThemeKind::Light);
    assert_eq!(
        theme.ui[UiRole::SelectionLight],
        builtin::default_theme(ThemeKind::Light).ui[UiRole::SelectionLight]
    );
}

#[test]
fn a_file_without_a_kind_is_still_incomplete() {
    let text = old_file(ThemeKind::Dark, "old").replace("kind = \"dark\"\n", "");
    let mut registry = themes::Registry::with_builtin();
    registry.insert_toml(&text, "old.toml").unwrap();
    assert!(matches!(
        registry.resolve("old"),
        Err(themes::ThemeError::Incomplete { .. })
    ));
}

#[test]
fn an_import_records_its_base() {
    let dir = tempfile::tempdir().unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plugin");
    let files = themes::catalog::import_into(&source, dir.path()).unwrap();
    for item in &files {
        assert_eq!(
            item.file.theme.base.as_deref(),
            Some(item.report.base.as_str())
        );
        let text = std::fs::read_to_string(&item.path).unwrap();
        assert!(text.contains("base = "), "{text}");
    }
}
