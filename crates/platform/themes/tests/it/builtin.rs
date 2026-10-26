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
    // Sorted by id.
    let mut expected = [
        builtin::ISLANDS_DARK_ID.to_owned(),
        builtin::ISLANDS_LIGHT_ID.to_owned(),
        builtin::dark_id(),
        builtin::light_id(),
    ];
    expected.sort();
    assert_eq!(ids, expected);
    for kind in [ThemeKind::Dark, ThemeKind::Light] {
        let theme = builtin::default_theme(kind);
        assert_eq!(theme.kind, kind);
        assert_eq!(theme.id, builtin::default_id_for(kind));
    }
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
fn islands_themes_have_the_file_status_colours_of_the_reference_schemes() {
    // `FILESTATUS_*` of the reference's Islands schemes (dark, light). The
    // ignored colour is orange there: names ignored outside excluded
    // folders are not drawn with it (they are grey, see `vcs-ui`).
    let expected = [
        (
            ThemeKind::Dark,
            [
                (UiRole::GitAdded, "#73bd79"),
                (UiRole::GitModified, "#70aeff"),
                (UiRole::GitDeleted, "#6f737a"),
                (UiRole::GitRenamed, "#70aeff"),
                (UiRole::GitUntracked, "#e88f89"),
                (UiRole::GitIgnored, "#d69a6b"),
                (UiRole::GitConflict, "#de6a66"),
                (UiRole::GitMerged, "#cf84cf"),
            ],
        ),
        (
            ThemeKind::Light,
            [
                (UiRole::GitAdded, "#067d17"),
                (UiRole::GitModified, "#0033b3"),
                (UiRole::GitDeleted, "#6c707e"),
                (UiRole::GitRenamed, "#0033b3"),
                (UiRole::GitUntracked, "#b23247"),
                (UiRole::GitIgnored, "#8c4f00"),
                (UiRole::GitConflict, "#de1b2e"),
                (UiRole::GitMerged, "#643cb8"),
            ],
        ),
    ];
    for (kind, roles) in expected {
        let theme = builtin::default_theme(kind);
        for (role, hex) in roles {
            assert_eq!(theme.ui[role].to_hex(), hex, "{} {role:?}", theme.id);
        }
    }
}

#[test]
fn generated_themes_underline_only_with_an_effect_colour() {
    // The reference paints an effect only when the scheme gives it a colour;
    // the islands themes come from its schemes, so every underline in them
    // names its colour rather than falling back to the text colour.
    for kind in [ThemeKind::Dark, ThemeKind::Light] {
        let theme = builtin::default_theme(kind);
        for (role, style) in theme.text.iter() {
            assert!(
                style.underline.is_none() || style.underline_color.is_some(),
                "{} underlines {role} in the text colour",
                theme.id
            );
        }
    }
    // The light scheme's warning sets an effect type but no effect colour:
    // only its background is drawn.
    let warning = &builtin::default_theme(ThemeKind::Light).text[TextRole::DiagnosticWarning];
    assert!(warning.bg.is_some());
    assert_eq!(warning.underline, None);
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
