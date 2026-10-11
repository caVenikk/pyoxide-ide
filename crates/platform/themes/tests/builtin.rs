use themes::contrast::{check_theme, failures};
use themes::{Registry, TextRole, ThemeKind, UiRole, builtin};

fn report(theme: &themes::Theme) -> String {
    failures(theme)
        .iter()
        .map(|r| {
            format!(
                "  {:.2}:1 (needs {:.1}) {}",
                r.ratio,
                r.need.ratio(),
                r.label
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dark_theme_passes_wcag_aa() {
    let theme = builtin::dark();
    assert_eq!(theme.kind, ThemeKind::Dark);
    let bad = report(&theme);
    assert!(bad.is_empty(), "contrast failures in {}:\n{bad}", theme.id);
}

#[test]
fn light_theme_passes_wcag_aa() {
    let theme = builtin::light();
    assert_eq!(theme.kind, ThemeKind::Light);
    let bad = report(&theme);
    assert!(bad.is_empty(), "contrast failures in {}:\n{bad}", theme.id);
}

#[test]
fn contrast_check_covers_many_pairs() {
    for kind in [ThemeKind::Dark, ThemeKind::Light] {
        let pairs = check_theme(&builtin::theme(kind));
        assert!(pairs.len() > 140, "{} pairs", pairs.len());
    }
}

#[test]
fn builtin_themes_are_registered_and_distinct() {
    let registry = Registry::with_builtin();
    let ids: Vec<&str> = registry.ids().collect();
    assert_eq!(ids, [builtin::DARK_ID, builtin::LIGHT_ID]);
    let dark = builtin::dark();
    let light = builtin::light();
    assert_ne!(
        dark.ui[UiRole::EditorBackground],
        light.ui[UiRole::EditorBackground]
    );
    assert!(dark.ui[UiRole::EditorBackground].luminance() < 0.1);
    assert!(light.ui[UiRole::EditorBackground].luminance() > 0.8);
}

#[test]
fn every_syntax_token_sets_a_style_or_is_plain_on_purpose() {
    // Roles a theme may leave plain (they render in the editor text colour).
    const PLAIN: &[TextRole] = &[TextRole::Variable, TextRole::Module];
    for theme in [builtin::dark(), builtin::light()] {
        for (role, style) in theme.text.iter() {
            assert!(
                !style.is_plain() || PLAIN.contains(&role),
                "{} leaves {role} unstyled",
                theme.id
            );
        }
    }
}

#[test]
#[ignore = "prints the tightest pairs of both themes"]
fn print_tightest_pairs() {
    for theme in [builtin::dark(), builtin::light()] {
        let mut pairs = check_theme(&theme);
        pairs.sort_by(|a, b| (a.ratio / a.need.ratio()).total_cmp(&(b.ratio / b.need.ratio())));
        println!("{}", theme.id);
        for p in pairs.iter().take(12) {
            println!(
                "  {:.2}:1 (needs {:.1}) {}",
                p.ratio,
                p.need.ratio(),
                p.label
            );
        }
    }
}

#[test]
fn theme_survives_a_round_trip_through_a_standalone_file() {
    let registry = Registry::with_builtin();
    for theme in [builtin::dark(), builtin::light()] {
        let text = theme.to_file().to_toml();
        let file = themes::ThemeFile::from_toml(&text, "round-trip").unwrap();
        assert!(file.theme.extends.is_none());
        let again = registry.resolve_file(&file).unwrap();
        assert_eq!(again, theme);
    }
}
