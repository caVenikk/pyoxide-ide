//! Importer tests against synthetic fixtures (invented colours).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use themes::import::{
    ImportError, Imported, KeySource, Level, Library, SourceKind, WarningKind, import_path,
};
use themes::{Registry, Rgba, TextRole, TextStyle, ThemeKind, UiRole, UnderlineStyle};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn rgba(hex: &str) -> Rgba {
    Rgba::parse(hex).unwrap()
}

fn by_name<'a>(items: &'a [Imported], name: &str) -> &'a Imported {
    items
        .iter()
        .find(|i| i.theme.name == name)
        .unwrap_or_else(|| panic!("no import named {name}"))
}

fn skyline(with_parent: bool) -> Imported {
    let mut library = Library::new();
    if with_parent {
        library
            .add_path(&fixtures().join("schemes/BaseNight.icls"), false)
            .unwrap();
    }
    library
        .add_path(&fixtures().join("schemes/Skyline.icls"), true)
        .unwrap();
    let mut all = library.import_all();
    assert_eq!(all.len(), 1);
    all.remove(0)
}

fn warnings(item: &Imported) -> Vec<&themes::import::Warning> {
    item.report.warnings.iter().collect()
}

// ---- colour schemes ---------------------------------------------------

#[test]
fn scheme_colours_attributes_and_inheritance() {
    let item = skyline(true);
    let t = &item.theme;
    assert_eq!(t.name, "Skyline");
    assert_eq!(t.id, "skyline");
    assert_eq!(t.kind, ThemeKind::Dark);
    assert_eq!(item.report.kind, SourceKind::Scheme);
    assert_eq!(item.report.base, "pyoxide-dark");

    // <colors>; the child overrides the parent's CARET_ROW_COLOR, the parent
    // supplies INDENT_GUIDE, "0" is black and 8 digits are RRGGBBAA.
    assert_eq!(t.ui[UiRole::CurrentLine], rgba("#12263a"));
    assert_eq!(t.ui[UiRole::IndentGuide], rgba("#2b4157"));
    assert_eq!(t.ui[UiRole::Caret], Rgba::BLACK);
    assert_eq!(t.ui[UiRole::Gutter], rgba("#0b1a27"));
    assert_eq!(t.ui[UiRole::LineNumber], rgba("#6f8aa6"));
    assert_eq!(t.ui[UiRole::EditorSelection], rgba("#2a4b6c"));
    assert_eq!(t.ui[UiRole::GutterAdded], rgba("#3fae68"));
    assert_eq!(t.ui[UiRole::GitAdded], rgba("#55c47f"));
    assert_eq!(t.ui[UiRole::Scrollbar], Rgba::new(0x8a, 0xa0, 0xb8, 0x40));
    assert_eq!(t.ui[UiRole::TerminalBackground], rgba("#0a1824"));
    assert_eq!(t.ui[UiRole::TerminalRed], rgba("#f06c6c"));

    // TEXT gives the editor defaults, the diagnostics take the effect colour.
    assert_eq!(t.ui[UiRole::EditorBackground], rgba("#0b1a27"));
    assert_eq!(t.ui[UiRole::EditorText], rgba("#c9d6e3"));
    assert_eq!(t.ui[UiRole::DiagnosticError], rgba("#f26d6d"));

    // Attributes: the child's keyword wins over the parent's, the number
    // and the class come from the parent.
    let keyword = &t.text[TextRole::Keyword];
    assert_eq!(keyword.fg, Some(rgba("#e0915b")));
    assert!(keyword.is_bold());
    assert_eq!(t.text[TextRole::Number].fg, Some(rgba("#66c2d6")));
    assert_eq!(t.text[TextRole::Class].fg, Some(rgba("#e8cf7a")));
    assert!(t.text[TextRole::Class].is_bold());
    assert_eq!(t.text[TextRole::ClassDecl].fg, Some(rgba("#e8cf7a")));
    assert_eq!(t.text[TextRole::String].fg, Some(rgba("#8fc77a")));
    assert_eq!(t.text[TextRole::Builtin].fg, Some(rgba("#4fc1b0")));
    let this = &t.text[TextRole::SelfParameter];
    assert_eq!(this.fg, Some(rgba("#d98aa8")));
    assert!(this.is_italic());
}

#[test]
fn base_attributes_are_followed() {
    let t = skyline(true).theme;
    // DEFAULT_DOC_COMMENT: own foreground, font style of DEFAULT_LINE_COMMENT.
    let doc = &t.text[TextRole::Docstring];
    assert_eq!(doc.fg, Some(rgba("#7b9b86")));
    assert!(doc.is_italic());
    let comment = &t.text[TextRole::Comment];
    assert_eq!(comment.fg, Some(rgba("#6b8299")));
    assert!(comment.is_italic());
    // PY.DECORATOR is only a reference; PY.ANNOTATION overrides the colour.
    assert_eq!(t.text[TextRole::Decorator].fg, Some(rgba("#c59cf2")));
    assert_eq!(t.text[TextRole::Annotation].fg, Some(rgba("#5fc9c0")));
}

#[test]
fn effects_become_underlines_strikeouts_and_links() {
    let t = skyline(true).theme;
    let error = &t.text[TextRole::DiagnosticError];
    assert_eq!(error.underline, Some(UnderlineStyle::Wavy));
    assert_eq!(error.underline_color, Some(rgba("#f26d6d")));
    assert!(t.text[TextRole::DiagnosticDeprecated].is_strikethrough());
    let link = &t.text[TextRole::Link];
    assert_eq!(link.fg, Some(rgba("#5aa0ff")));
    assert_eq!(link.underline, Some(UnderlineStyle::Single));
}

#[test]
fn an_effect_type_without_an_effect_colour_draws_nothing() {
    // Every effect type without a colour: the reference paints no effect
    // then, so the styles keep only their colours.
    let xml = r#"<scheme name="Effectless Fixture" version="142">
        <attributes>
          <option name="TEXT"><value>
            <option name="FOREGROUND" value="23282d" />
            <option name="BACKGROUND" value="fbfaf5" />
          </value></option>
          <option name="FOLDED_TEXT_ATTRIBUTES"><value>
            <option name="FOREGROUND" value="4a5a4a" />
            <option name="EFFECT_TYPE" value="0" />
          </value></option>
          <option name="WARNING_ATTRIBUTES"><value>
            <option name="BACKGROUND" value="faf0cc" />
            <option name="EFFECT_TYPE" value="1" />
          </value></option>
          <option name="IDENTIFIER_UNDER_CARET_ATTRIBUTES"><value>
            <option name="BACKGROUND" value="e0e8f0" />
            <option name="EFFECT_TYPE" value="2" />
          </value></option>
          <option name="DEPRECATED_ATTRIBUTES"><value>
            <option name="FOREGROUND" value="606060" />
            <option name="EFFECT_TYPE" value="3" />
          </value></option>
          <option name="WRITE_IDENTIFIER_UNDER_CARET_ATTRIBUTES"><value>
            <option name="BACKGROUND" value="f0e0e8" />
            <option name="EFFECT_TYPE" value="4" />
          </value></option>
          <option name="HYPERLINK_ATTRIBUTES"><value>
            <option name="FOREGROUND" value="2050c0" />
            <option name="EFFECT_TYPE" value="5" />
          </value></option>
          <option name="ERRORS_ATTRIBUTES"><value>
            <option name="EFFECT_COLOR" value="d03030" />
            <option name="EFFECT_TYPE" value="2" />
          </value></option>
        </attributes></scheme>"#;
    let mut library = Library::new();
    library
        .add_scheme_xml(xml, "inline", "inline.icls", true)
        .unwrap();
    let item = library.import_all().remove(0);
    let text = &item.theme.text;
    let only = |fg: Option<&str>, bg: Option<&str>| TextStyle {
        fg: fg.map(rgba),
        bg: bg.map(rgba),
        ..TextStyle::default()
    };
    assert_eq!(text[TextRole::FoldedText], only(Some("#4a5a4a"), None));
    assert_eq!(
        text[TextRole::DiagnosticWarning],
        only(None, Some("#faf0cc"))
    );
    assert_eq!(text[TextRole::IdentifierRead], only(None, Some("#e0e8f0")));
    assert_eq!(
        text[TextRole::DiagnosticDeprecated],
        only(Some("#606060"), None)
    );
    assert_eq!(text[TextRole::IdentifierWrite], only(None, Some("#f0e0e8")));
    assert_eq!(text[TextRole::Link], only(Some("#2050c0"), None));
    // With a colour the effect is drawn.
    let error = &text[TextRole::DiagnosticError];
    assert_eq!(error.underline, Some(UnderlineStyle::Wavy));
    assert_eq!(error.underline_color, Some(rgba("#d03030")));
    // Drawing nothing is what the reference does: no approximation.
    assert!(
        item.report
            .approximations
            .iter()
            .all(|a| !a.note.contains("strikeout")),
        "{:?}",
        item.report.approximations
    );
}

#[test]
fn missing_tokens_follow_the_fallback_chain_before_the_base_theme() {
    let item = skyline(true);
    let t = &item.theme;
    // No PY.KEYWORD_ARGUMENT or PY.METHOD_CALL: they look like the parameter
    // and the function call the scheme does define.
    assert_eq!(t.text[TextRole::KeywordArgument].fg, Some(rgba("#d8c9a3")));
    assert_eq!(t.text[TextRole::MethodCall].fg, Some(rgba("#9dbde8")));
    let derived: Vec<&str> = item
        .report
        .approximations
        .iter()
        .filter(|a| a.note.contains("no key sets it"))
        .map(|a| a.target.as_str())
        .collect();
    assert!(derived.contains(&"syntax.keyword-argument"));
    assert!(derived.contains(&"syntax.method-call"));
    // A todo marker looks like the comment it sits in.
    assert_eq!(t.text[TextRole::Todo], t.text[TextRole::Comment]);
    // Nothing in the sources (or the fallback chain) sets the invalid escape
    // style: the base theme does.
    let base = themes::builtin::dark();
    assert_eq!(
        t.text[TextRole::StringEscapeInvalid],
        base.text[TextRole::StringEscapeInvalid]
    );
    assert!(
        item.report
            .defaulted
            .iter()
            .any(|d| d.target == "syntax.string-escape-invalid")
    );
}

#[test]
fn merged_tokens_and_unsupported_effects_are_approximations() {
    let item = skyline(true);
    // PY.PARENTHS wins; PY.BRACKETS differs and is merged into `bracket`.
    assert_eq!(item.theme.text[TextRole::Bracket].fg, Some(rgba("#b0bccb")));
    assert!(item.report.approximations.iter().any(|a| {
        a.key == "PY.BRACKETS" && a.target == "syntax.bracket" && a.note.contains("PY.PARENTHS")
    }));
    assert!(
        item.report
            .mapped
            .iter()
            .any(|m| m.key == "PY.BRACKETS" && !m.used)
    );
    // EFFECT_TYPE 9 does not exist.
    assert!(
        item.report
            .approximations
            .iter()
            .any(|a| a.key == "TYPO" && a.note.contains("EFFECT_TYPE 9"))
    );
}

#[test]
fn fonts_are_imported_in_front_of_the_default_families() {
    let t = skyline(true).theme;
    let base = themes::builtin::dark();
    assert_eq!(t.fonts.editor_family[0], "Fixture Mono");
    assert_eq!(&t.fonts.editor_family[1..], &base.fonts.editor_family[..]);
    assert_eq!(t.fonts.editor_size, 15.0);
    assert!((t.fonts.editor_line_spacing - 1.3).abs() < 1e-6);
    assert!(t.fonts.editor_ligatures);
    assert_eq!(t.fonts.terminal_family[0], "Fixture Console");
    assert_eq!(t.fonts.terminal_size, 12.0);
    assert_eq!(t.fonts.ui_size, base.fonts.ui_size);
}

#[test]
fn unmapped_keys_are_listed_with_counts() {
    let item = skyline(true);
    let find = |source: KeySource, key: &str| {
        item.report
            .unmapped
            .iter()
            .find(|u| u.source == source && u.key == key)
    };
    assert_eq!(
        find(KeySource::SchemeColor, "FIXTURE_ONLY_COLOR")
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        find(KeySource::SchemeAttribute, "FIXTURE.ONLY_KEY")
            .unwrap()
            .count,
        1
    );
    assert_eq!(
        find(KeySource::SchemeOption, "FONT_SCALE").unwrap().count,
        1
    );
    // Present but empty: there is no role for the selection foreground.
    assert!(find(KeySource::SchemeColor, "SELECTION_FOREGROUND").is_some());
    // A mapped key is not listed.
    assert!(find(KeySource::SchemeColor, "CARET_COLOR").is_none());
    let groups = item.report.unmapped_groups();
    assert!(groups.iter().any(|(g, _)| g == "FIXTURE"));
    let summary = item.report.summary();
    assert!(summary.mapped > 30);
    assert!(summary.unmapped_keys >= 4);
}

#[test]
fn a_missing_parent_scheme_is_reported_and_filled_from_the_base_theme() {
    let item = skyline(false);
    let warn = item
        .report
        .warnings
        .iter()
        .find(|w| matches!(&w.kind, WarningKind::ParentSchemeUnavailable { name, .. } if name == "Base Night"))
        .expect("warning about the parent");
    // "Base Night" is not a base scheme of the IDE: a real warning.
    assert_eq!(warn.level, Level::Warning);
    assert!(matches!(
        warn.kind,
        WarningKind::ParentSchemeUnavailable { known: false, .. }
    ));
    // The number would have come from the parent; the base theme supplies it.
    let base = themes::builtin::dark();
    assert_eq!(
        item.theme.text[TextRole::Number],
        base.text[TextRole::Number]
    );
    assert!(
        item.report
            .defaulted
            .iter()
            .any(|d| d.target == "syntax.number")
    );
    // Own values are still imported.
    assert_eq!(item.theme.text[TextRole::Keyword].fg, Some(rgba("#e0915b")));
    assert!(item.report.summary().warnings >= 1);
}

#[test]
fn a_scheme_for_a_light_background_selects_the_light_base() {
    let xml = r#"<scheme name="Daylight Fixture" version="142">
        <attributes>
          <option name="TEXT"><value>
            <option name="FOREGROUND" value="23282d" />
            <option name="BACKGROUND" value="fbfaf5" />
          </value></option>
        </attributes></scheme>"#;
    let mut library = Library::new();
    library
        .add_scheme_xml(xml, "inline", "inline.icls", true)
        .unwrap();
    let item = library.import_all().remove(0);
    assert_eq!(item.theme.kind, ThemeKind::Light);
    assert_eq!(item.report.base, "pyoxide-light");
    assert_eq!(item.theme.ui[UiRole::EditorBackground], rgba("#fbfaf5"));
}

// ---- UI themes --------------------------------------------------------

fn plugin_imports() -> Vec<Imported> {
    import_path(&fixtures().join("plugin"), &[]).unwrap()
}

#[test]
fn directory_import_finds_every_theme_once() {
    let items = plugin_imports();
    let mut names: Vec<&str> = items.iter().map(|i| i.theme.name.as_str()).collect();
    names.sort();
    // The scheme file Nightfall.xml is used by the themes, not imported alone;
    // the metadata JSON is not a theme.
    assert_eq!(names, ["Dusk", "Nightfall", "Orphan", "Plain"]);
}

#[test]
fn wildcard_keys_palette_names_and_numbers() {
    let items = plugin_imports();
    let item = by_name(&items, "Nightfall");
    let t = &item.theme;
    assert_eq!(t.kind, ThemeKind::Dark);
    assert_eq!(t.author.as_deref(), Some("Fixture Author"));
    // A specific key beats the wildcard; a palette name resolves through
    // another palette name; wildcard keys supply the rest.
    assert_eq!(t.ui[UiRole::Surface], rgba("#0a1722"));
    assert_eq!(t.ui[UiRole::SurfaceAlt], rgba("#0a1722"));
    assert_eq!(t.ui[UiRole::SurfaceRaised], rgba("#1b3147"));
    assert_eq!(t.ui[UiRole::Input], rgba("#142433"));
    assert_eq!(t.ui[UiRole::Text], rgba("#cfdbe8"));
    assert_eq!(t.ui[UiRole::TextMuted], rgba("#86a0b8"));
    assert_eq!(t.ui[UiRole::TextDisabled], rgba("#5c7287"));
    assert_eq!(t.ui[UiRole::Selection], rgba("#2d5a87"));
    assert_eq!(t.ui[UiRole::SelectionInactive], rgba("#243f58"));
    assert_eq!(t.ui[UiRole::Border], rgba("#2a4259"));
    // #RRGGBBAA keeps the alpha.
    assert_eq!(t.ui[UiRole::Hover], Rgba::new(0x1f, 0x3a, 0x52, 0xcc));
    assert_eq!(t.ui[UiRole::Accent], rgba("#e3702d"));
    assert_eq!(t.ui[UiRole::OnAccent], rgba("#10100f"));
    assert_eq!(t.ui[UiRole::Focus], rgba("#ffb347"));
    assert_eq!(t.ui[UiRole::TextLink], rgba("#5fb0ff"));
    assert_eq!(t.ui[UiRole::Danger], rgba("#ff6b6b"));
    assert_eq!(t.ui[UiRole::StatusBar], rgba("#0e1f2e"));
    assert_eq!(t.ui[UiRole::StatusBarText], rgba("#86a0b8"));
    // Numbers: the arc is a diameter.
    assert_eq!(t.metrics[themes::Metric::RowHeight], 26.0);
    assert_eq!(t.metrics[themes::Metric::CornerRadius], 6.0);
}

#[test]
fn editor_scheme_comes_from_the_editor_scheme_path() {
    let items = plugin_imports();
    let item = by_name(&items, "Nightfall");
    let t = &item.theme;
    assert_eq!(t.ui[UiRole::EditorBackground], rgba("#0c1b29"));
    assert_eq!(t.ui[UiRole::EditorText], rgba("#cdd9e6"));
    assert_eq!(t.ui[UiRole::Gutter], rgba("#0d1d2c"));
    assert_eq!(t.text[TextRole::Keyword].fg, Some(rgba("#e8793a")));
    assert_eq!(t.text[TextRole::String].fg, Some(rgba("#9acd6f")));
    assert_eq!(t.text[TextRole::Builtin].fg, Some(rgba("#52c7b8")));
    assert!(
        item.report
            .sources
            .iter()
            .any(|s| s.ends_with("Nightfall.xml"))
    );
    assert!(
        item.report
            .sources
            .iter()
            .any(|s| s.ends_with("Nightfall.theme.json"))
    );
}

#[test]
fn known_parent_without_file_is_a_note_and_missing_values_come_from_the_base() {
    let items = plugin_imports();
    let item = by_name(&items, "Nightfall");
    let note = item
        .report
        .warnings
        .iter()
        .find(|w| matches!(&w.kind, WarningKind::ParentThemeUnavailable { id, .. } if id == "ExperimentalDark"))
        .expect("note about the known parent");
    assert_eq!(note.level, Level::Info);
    if let WarningKind::ParentThemeUnavailable { chain, .. } = &note.kind {
        assert_eq!(chain, &["ExperimentalDark", "Darcula"]);
    }
    // Known base scheme not given: a note as well.
    assert!(item.report.warnings.iter().any(|w| {
        w.level == Level::Info
            && matches!(&w.kind, WarningKind::ParentSchemeUnavailable { name, known: true } if name == "Darcula")
    }));
    // Neither the theme nor the scheme sets the diff colours.
    let base = themes::builtin::dark();
    assert_eq!(item.theme.ui[UiRole::DiffAdded], base.ui[UiRole::DiffAdded]);
    assert!(
        item.report
            .defaulted
            .iter()
            .any(|d| d.target == "ui.diff-added")
    );
}

#[test]
fn unknown_parent_theme_is_a_warning() {
    let items = plugin_imports();
    let item = by_name(&items, "Orphan");
    assert_eq!(item.theme.kind, ThemeKind::Light);
    assert_eq!(item.report.base, "pyoxide-light");
    assert_eq!(item.theme.ui[UiRole::Surface], rgba("#f4efe6"));
    assert_eq!(item.theme.ui[UiRole::Text], rgba("#2b2723"));
    let warn = warnings(item)
        .into_iter()
        .find(|w| matches!(&w.kind, WarningKind::UnknownParentTheme { id } if id == "Mystery"))
        .expect("unknown parent warning");
    assert_eq!(warn.level, Level::Warning);
    assert!(warn.message.contains("Mystery"));
}

#[test]
fn unresolved_palette_names_are_warnings() {
    let items = plugin_imports();
    let item = by_name(&items, "Nightfall");
    assert!(item.report.warnings.iter().any(|w| matches!(
        &w.kind,
        WarningKind::UnresolvedPaletteName { name, .. } if name == "no-such-name"
    )));
    // The failed key does not block the role: the base theme supplies it.
    let base = themes::builtin::dark();
    assert_eq!(item.theme.ui[UiRole::Warning], base.ui[UiRole::Warning]);
}

#[test]
fn unmapped_ui_keys_and_sections_are_counted() {
    let items = plugin_imports();
    let item = by_name(&items, "Nightfall");
    let unmapped = &item.report.unmapped;
    let find =
        |source: KeySource, key: &str| unmapped.iter().find(|u| u.source == source && u.key == key);
    assert!(find(KeySource::UiTheme, "Quirky.flag").is_some());
    assert!(find(KeySource::UiTheme, "Quirky.list").is_some());
    assert!(find(KeySource::UiTheme, "Quirky.Deep.value").is_some());
    assert_eq!(find(KeySource::UiThemeSection, "icons").unwrap().count, 2);
    // Keys that the tables know, directly or through a wildcard, are not listed.
    assert!(find(KeySource::UiTheme, "Panel.background").is_none());
    assert!(find(KeySource::UiTheme, "*.background").is_none());
    assert!(find(KeySource::UiTheme, "*.arc").is_none());
    let groups = item.report.unmapped_groups();
    assert_eq!(groups[0], ("Quirky".to_owned(), 3));
}

#[test]
fn child_theme_inherits_ui_keys_and_palette_overrides_apply() {
    let items = plugin_imports();
    let item = by_name(&items, "Dusk");
    let t = &item.theme;
    assert_eq!(t.kind, ThemeKind::Dark, "dark is inherited");
    // `deep` is redefined by the child, and the parent's Panel.background
    // refers to it by name.
    assert_eq!(t.ui[UiRole::Surface], rgba("#1a0f2e"));
    assert_eq!(t.ui[UiRole::StatusBar], rgba("#2a1a45"));
    // A flat dotted key in the child.
    assert_eq!(t.ui[UiRole::TextDisabled], rgba("#7a6a99"));
    // Inherited from the parent.
    assert_eq!(t.ui[UiRole::Accent], rgba("#e3702d"));
    assert_eq!(t.metrics[themes::Metric::RowHeight], 26.0);
    // The editor scheme of the parent applies.
    assert_eq!(t.text[TextRole::Keyword].fg, Some(rgba("#e8793a")));
    // The chain ends at a parent that is a known id without a file.
    assert!(item.report.warnings.iter().any(|w| matches!(
        &w.kind,
        WarningKind::ParentThemeUnavailable { id, .. } if id == "ExperimentalDark"
    )));
    assert!(item.report.sources.len() >= 3);
}

#[test]
fn json_without_theme_suffix_and_scheme_by_name() {
    let items = plugin_imports();
    let item = by_name(&items, "Plain");
    // editorScheme is a scheme name here, not a path.
    assert_eq!(item.theme.text[TextRole::Keyword].fg, Some(rgba("#e8793a")));
    // The scheme's TEXT background beats the theme's Editor.background.
    assert_eq!(item.theme.ui[UiRole::EditorBackground], rgba("#0c1b29"));
    assert!(item.report.mapped.iter().any(|m| {
        m.key == "Editor.background" && m.target == "ui.editor-background" && !m.used
    }));
}

#[test]
fn single_theme_file_finds_its_scheme_next_to_it() {
    let mut library = Library::new();
    library
        .add_path(&fixtures().join("plugin/themes/Nightfall.theme.json"), true)
        .unwrap();
    let all = library.import_all();
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].theme.text[TextRole::Keyword].fg,
        Some(rgba("#e8793a"))
    );
}

#[test]
fn base_schemes_from_the_context_fill_missing_values() {
    let plugin = fixtures().join("plugin");
    let with_base = import_path(&plugin, &[fixtures().join("base")]).unwrap();
    let item = by_name(&with_base, "Nightfall");
    // Darcula (from the context) supplies the number, which Nightfall.xml omits.
    assert_eq!(item.theme.text[TextRole::Number].fg, Some(rgba("#5fb3c9")));
    assert!(
        !item
            .report
            .warnings
            .iter()
            .any(|w| matches!(&w.kind, WarningKind::ParentSchemeUnavailable { .. }))
    );
    let without = plugin_imports();
    let before = by_name(&without, "Nightfall");
    assert!(item.report.defaulted.len() < before.report.defaulted.len());
    // The context files are not imported themselves.
    assert_eq!(with_base.len(), 4);
}

#[test]
fn a_theme_without_editor_scheme_uses_the_default_scheme_for_its_brightness() {
    let plugin = fixtures().join("plugin");
    let items = import_path(&plugin, &[fixtures().join("base")]).unwrap();
    let orphan = by_name(&items, "Orphan");
    // Light theme: the base scheme "Default" supplies the editor background.
    assert_eq!(orphan.theme.ui[UiRole::EditorBackground], rgba("#fbfcfd"));
    assert_eq!(
        orphan.theme.text[TextRole::Keyword].fg,
        Some(rgba("#993300"))
    );
    assert!(orphan.report.warnings.iter().any(|w| {
        w.level == Level::Info
            && matches!(&w.kind, WarningKind::SchemeAssumed { name } if name == "Default")
    }));
    // Without the base schemes nothing is assumed and nothing is reported.
    let alone = plugin_imports();
    let orphan = by_name(&alone, "Orphan");
    assert!(
        !orphan
            .report
            .warnings
            .iter()
            .any(|w| matches!(w.kind, WarningKind::SchemeAssumed { .. }))
    );
}

#[test]
fn a_missing_known_parent_does_not_hide_its_available_ancestors() {
    let mut library = Library::new();
    library
        .add_ui_theme_json(
            r##"{"name": "Darcula", "dark": true, "ui": {"Panel": {"background": "#101a22"}}}"##,
            "inline-root",
            "themes/darcula.theme.json",
            false,
        )
        .unwrap();
    library
        .add_ui_theme_json(
            r##"{"name": "Leaf", "parentTheme": "ExperimentalDark", "ui": {"Label": {"foreground": "#e0e6ec"}}}"##,
            "inline-leaf",
            "themes/leaf.theme.json",
            true,
        )
        .unwrap();
    let item = library.import_all().remove(0);
    // ExperimentalDark is missing, its table parent Darcula is not.
    assert_eq!(item.theme.ui[UiRole::Surface], rgba("#101a22"));
    assert_eq!(item.theme.ui[UiRole::Text], rgba("#e0e6ec"));
    assert_eq!(item.theme.kind, ThemeKind::Dark);
    let notes: Vec<_> = item
        .report
        .warnings
        .iter()
        .filter(|w| matches!(w.kind, WarningKind::ParentThemeUnavailable { .. }))
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(item.report.sources.len(), 2);
}

// ---- archives ---------------------------------------------------------

fn add_dir_to_zip(zip: &mut zip::ZipWriter<File>, root: &Path, dir: &Path) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let name = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        if path.is_dir() {
            zip.add_directory(format!("{name}/"), options).unwrap();
            add_dir_to_zip(zip, root, &path);
        } else {
            zip.start_file(name, options).unwrap();
            zip.write_all(&std::fs::read(&path).unwrap()).unwrap();
        }
    }
}

fn build_jar(target: &Path, extra: &[(&str, &str)]) {
    let root = fixtures().join("plugin");
    let mut zip = zip::ZipWriter::new(File::create(target).unwrap());
    add_dir_to_zip(&mut zip, &root, &root);
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in extra {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn archive_import_reads_data_files_and_never_class_files() {
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("fixture-plugin.jar");
    build_jar(
        &jar,
        &[
            // Path tricks and executables must not be read either.
            ("../escape.theme.json", r#"{"name":"Escape","ui":{}}"#),
            ("themes/../up.theme.json", r#"{"name":"Up","ui":{}}"#),
            ("lib/helper.jar", "not an archive"),
        ],
    );
    let mut library = Library::new();
    library.add_path(&jar, true).unwrap();
    let summary = &library.archives()[0];
    assert!(summary.entries >= 12, "{summary:?}");
    assert_eq!(summary.ignored_classes, 2);
    let mut read = summary.read.clone();
    read.sort();
    assert_eq!(
        read,
        [
            "themes/Dusk.theme.json",
            "themes/Nightfall.theme.json",
            "themes/Nightfall.xml",
            "themes/Orphan.theme.json",
            "themes/Plain.json",
        ]
    );
    assert!(
        read.iter()
            .all(|n| !n.to_ascii_lowercase().ends_with(".class")),
        "a class file was opened"
    );
    assert!(library.skipped().is_empty(), "{:?}", library.skipped());

    let items = library.import_all();
    let mut names: Vec<&str> = items.iter().map(|i| i.theme.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Dusk", "Nightfall", "Orphan", "Plain"]);
}

#[test]
fn archive_and_directory_give_the_same_themes() {
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("p.jar");
    build_jar(&jar, &[]);
    let from_jar = import_path(&jar, &[]).unwrap();
    let from_dir = plugin_imports();
    assert_eq!(from_jar.len(), from_dir.len());
    for item in &from_dir {
        let twin = by_name(&from_jar, &item.theme.name);
        assert_eq!(twin.theme, item.theme, "{}", item.theme.name);
        assert_eq!(twin.report.summary(), item.report.summary());
    }
}

#[test]
fn scheme_referenced_by_path_outside_themes_is_read_from_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("ext.jar");
    let mut zip = zip::ZipWriter::new(File::create(&jar).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in [
        (
            "themes/Ext.theme.json",
            r#"{"name":"Ext","dark":true,"editorScheme":"/colors/Ext.xml","ui":{}}"#,
        ),
        (
            "colors/Ext.xml",
            r#"<scheme name="Ext Colors" version="142"><attributes>
                 <option name="DEFAULT_KEYWORD"><value><option name="FOREGROUND" value="c8a2f0"/></value></option>
               </attributes></scheme>"#,
        ),
        ("colors/Unreferenced.xml", r#"<scheme name="Never Read"/>"#),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();

    let mut library = Library::new();
    library.add_path(&jar, true).unwrap();
    let read = &library.archives()[0].read;
    assert!(read.iter().any(|n| n == "colors/Ext.xml"));
    assert!(!read.iter().any(|n| n.contains("Unreferenced")));
    let items = library.import_all();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].theme.text[TextRole::Keyword].fg,
        Some(rgba("#c8a2f0"))
    );
}

#[test]
fn oversized_and_broken_entries_are_skipped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("odd.jar");
    let mut zip = zip::ZipWriter::new(File::create(&jar).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in [
        (
            "themes/Good.theme.json",
            r#"{"name":"Good","dark":false,"ui":{}}"#,
        ),
        ("themes/NotATheme.json", r#"{"hello":"world"}"#),
        ("themes/Broken.theme.json", "{ not json"),
        ("themes/NotAScheme.xml", "<idea-plugin/>"),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let mut library = Library::new();
    library.add_path(&jar, true).unwrap();
    assert_eq!(library.skipped().len(), 3);
    let items = library.import_all();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].theme.kind, ThemeKind::Light);
}

// ---- errors and output ------------------------------------------------

#[test]
fn bad_inputs_fail_with_specific_errors() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("notes.txt");
    std::fs::write(&text, "x").unwrap();
    assert!(matches!(
        import_path(&text, &[]),
        Err(ImportError::Unsupported(_))
    ));
    let json = dir.path().join("broken.theme.json");
    std::fs::write(&json, "{").unwrap();
    assert!(matches!(
        import_path(&json, &[]),
        Err(ImportError::Parse { .. })
    ));
    let other = dir.path().join("settings.json");
    std::fs::write(&other, r#"{"editor.fontSize": 12}"#).unwrap();
    assert!(matches!(
        import_path(&other, &[]),
        Err(ImportError::Parse { .. })
    ));
    let zip = dir.path().join("broken.jar");
    std::fs::write(&zip, "PK nothing").unwrap();
    assert!(matches!(
        import_path(&zip, &[]),
        Err(ImportError::Archive { .. })
    ));
    assert!(matches!(
        import_path(&dir.path().join("missing.icls"), &[]),
        Err(ImportError::Io { .. })
    ));
    let empty = tempfile::tempdir().unwrap();
    assert!(matches!(
        import_path(empty.path(), &[]),
        Err(ImportError::Empty(_))
    ));
}

#[test]
fn imported_themes_round_trip_through_toml() {
    let registry = Registry::with_builtin();
    let mut all = plugin_imports();
    all.push(skyline(true));
    for item in all {
        let text = item.theme.to_file().to_toml();
        let file = themes::ThemeFile::from_toml(&text, &item.theme.name).unwrap();
        let again = registry.resolve_file(&file).unwrap();
        assert_eq!(again, item.theme, "{}", item.theme.name);
    }
}

#[test]
fn reports_render_and_serialize() {
    let item = skyline(false);
    let short = item.report.render(false);
    assert!(short.contains("Skyline"));
    assert!(short.contains("mapped"));
    assert!(short.contains("unmapped by group"));
    let long = item.report.render(true);
    assert!(long.contains("syntax.keyword"));
    assert!(long.contains("FIXTURE_ONLY_COLOR"));
    let json = serde_json::to_value(&item.report).unwrap();
    assert_eq!(json["name"], "Skyline");
    assert!(
        json["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["kind"] == "parent-scheme-unavailable")
    );
    assert!(json["mapped"].as_array().unwrap().len() > 10);
}

// ---- language attributes ------------------------------------------------

/// A language plugin's `<list>` of attributes for a base scheme applies to
/// every scheme that inherits from that base; the scheme's own entries win.
#[test]
fn language_attribute_lists_extend_their_base_scheme() {
    let dir = tempfile::tempdir().unwrap();
    let colors = dir.path().join("colorSchemes");
    std::fs::create_dir(&colors).unwrap();
    let write = |name: &str, text: &str| std::fs::write(colors.join(name), text).unwrap();
    write(
        "LangDarcula.xml",
        r#"<list>
  <option name="PY.BUILTIN_NAME"><value><option name="FOREGROUND" value="a1b2c3"/></value></option>
  <option name="PY.KEYWORD_ARGUMENT"><value><option name="FOREGROUND" value="111111"/></value></option>
</list>"#,
    );
    write(
        "Evening.xml",
        r#"<scheme name="Evening" version="142" parent_scheme="Darcula">
  <colors><option name="CARET_ROW_COLOR" value="202020"/></colors>
  <attributes>
    <option name="TEXT"><value><option name="FOREGROUND" value="dddddd"/><option name="BACKGROUND" value="101010"/></value></option>
    <option name="PY.KEYWORD_ARGUMENT"><value><option name="FOREGROUND" value="222222"/></value></option>
  </attributes>
</scheme>"#,
    );
    let mut library = Library::new();
    library.add_path(dir.path(), true).unwrap();
    let all = library.import_all();
    // The attribute list is not a theme of its own.
    let names: Vec<_> = all.iter().map(|i| i.theme.name.clone()).collect();
    assert_eq!(names, ["Evening"]);
    let t = &all[0].theme;
    assert_eq!(t.text[TextRole::Builtin].fg, Some(rgba("#a1b2c3")));
    assert_eq!(t.text[TextRole::KeywordArgument].fg, Some(rgba("#222222")));

    // A list whose file name names no known base scheme is skipped.
    write(
        "LangUnknown.xml",
        r#"<list><option name="PY.NUMBER"/></list>"#,
    );
    let mut library = Library::new();
    library.add_path(dir.path(), true).unwrap();
    assert!(
        library
            .skipped()
            .iter()
            .any(|s| s.path.ends_with("LangUnknown.xml")),
        "{:?}",
        library.skipped()
    );
}

// ---- the deprecated BACKGROUND colour -----------------------------------

/// A theme plugin whose scheme sets the editor background with the
/// deprecated `<colors>` option `BACKGROUND` and a different `TEXT`
/// background: the option wins, as in the reference, and is reported under
/// its own name.
#[test]
fn the_deprecated_background_colour_is_the_editor_background() {
    let dir = tempfile::tempdir().unwrap();
    let themes = dir.path().join("themes");
    std::fs::create_dir(&themes).unwrap();
    std::fs::write(
        themes.join("Ember.theme.json"),
        r##"{
  "name": "Ember",
  "dark": true,
  "parentTheme": "Islands Dark",
  "editorScheme": "/themes/Ember.xml",
  "ui": { "*": { "background": "#2a2420" } }
}"##,
    )
    .unwrap();
    std::fs::write(
        themes.join("Ember.xml"),
        r#"<scheme name="Ember" version="142" parent_scheme="Darcula">
  <colors>
    <option name="BACKGROUND" value="1e1a17"/>
    <option name="GUTTER_BACKGROUND" value="1e1a17"/>
    <option name="LINE_NUMBERS_COLOR" value="5c5148"/>
  </colors>
  <attributes>
    <option name="TEXT"><value>
      <option name="FOREGROUND" value="f2e8dc"/>
      <option name="BACKGROUND" value="3a322b"/>
      <option name="EFFECT_TYPE" value="5"/>
    </value></option>
  </attributes>
</scheme>"#,
    )
    .unwrap();
    let all = import_path(dir.path(), &[fixtures().join("base")]).unwrap();
    let item = by_name(&all, "Ember");
    let t = &item.theme;
    assert_eq!(t.ui[UiRole::EditorBackground], rgba("#1e1a17"));
    assert_eq!(t.ui[UiRole::EditorText], rgba("#f2e8dc"));
    assert_eq!(t.ui[UiRole::LineNumber], rgba("#5c5148"));
    // The scheme sets no EDITOR_GUTTER_BACKGROUND: the gutter is the editor.
    assert_eq!(t.ui[UiRole::Gutter], rgba("#1e1a17"));

    let mapped = |key: &str| {
        item.report
            .mapped
            .iter()
            .find(|m| m.key == key && m.target == "ui.editor-background")
    };
    let background = mapped("BACKGROUND").expect("BACKGROUND is reported");
    assert_eq!(background.source, KeySource::SchemeColor);
    assert!(background.used);
    assert!(
        mapped("TEXT.BACKGROUND").is_none(),
        "{:?}",
        item.report.mapped
    );
    assert!(
        !item.report.unmapped.iter().any(|u| u.key == "BACKGROUND"),
        "{:?}",
        item.report.unmapped
    );
    assert!(
        !item
            .report
            .warnings
            .iter()
            .any(|w| matches!(w.kind, WarningKind::TextFromBackground { .. }))
    );
}

/// The option of a scheme without a `TEXT` entry makes black text, also
/// over a parent's `TEXT`, and the report says so. A child's own `TEXT`
/// is not changed by its parent's option.
#[test]
fn the_deprecated_background_colour_without_text_and_across_parents() {
    let mut library = Library::new();
    library
        .add_scheme_xml(
            r#"<scheme name="Paper Base" version="142" parent_scheme="Default">
              <colors><option name="BACKGROUND" value="f6f1e7"/></colors></scheme>"#,
            "base",
            "base.icls",
            false,
        )
        .unwrap();
    library
        .add_scheme_xml(
            r#"<scheme name="Paper" version="142" parent_scheme="Paper Base">
              <attributes><option name="TEXT"><value>
                <option name="FOREGROUND" value="2b2620"/><option name="BACKGROUND" value="fffdf8"/>
              </value></option></attributes></scheme>"#,
            "child",
            "child.icls",
            true,
        )
        .unwrap();
    library
        .add_path(
            &fixtures().join("base/DefaultColorSchemesManager.xml"),
            false,
        )
        .unwrap();
    let child = library.import_all().remove(0);
    assert_eq!(child.theme.name, "Paper");
    assert_eq!(child.theme.ui[UiRole::EditorBackground], rgba("#fffdf8"));
    assert_eq!(child.theme.ui[UiRole::EditorText], rgba("#2b2620"));
    assert_eq!(child.theme.kind, ThemeKind::Light);

    let base = library
        .schemes()
        .find(|s| s.name == "Paper Base")
        .unwrap()
        .clone();
    let item = library.import_scheme(&base);
    assert_eq!(item.theme.ui[UiRole::EditorBackground], rgba("#f6f1e7"));
    // Not the Default scheme's text colour: the reference makes it black.
    assert_eq!(item.theme.ui[UiRole::EditorText], Rgba::BLACK);
    let note = item
        .report
        .warnings
        .iter()
        .find(|w| matches!(&w.kind, WarningKind::TextFromBackground { scheme } if scheme == "Paper Base"))
        .expect("a note on the black text");
    assert_eq!(note.level, Level::Info);
}
