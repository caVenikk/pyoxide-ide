//! Turns a resolved set of inputs (a UI theme chain and/or a scheme chain)
//! into a Pyoxide theme and a report.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::builtin;
use crate::color::Rgba;
use crate::icon_palette::{control_default, reference_color};
use crate::model::{Controls, Theme, ThemeKind, UnderlineStyle};
use crate::roles::{Metric, TextRole, UiRole};
use crate::{TextStyle, TextStyles, UiColors};

use super::mapping::{
    self, Src, known_keys, metric_sources, text_sources, ui_sources, wildcard_property,
};
use super::report::{
    Approximation, Defaulted, ImportReport, KeySource, Level, MappedKey, SourceKind, UnmappedKey,
    Warning, WarningKind,
};
use super::scheme::{Attribute, BACKGROUND_OPTION, BackgroundOption, Scheme, SchemeChain, TEXT};
use super::ui_theme::UiThemeData;

/// A finished import.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub theme: Theme,
    pub report: ImportReport,
}

pub(crate) struct Inputs<'a> {
    pub name: String,
    pub kind: SourceKind,
    pub author: Option<String>,
    pub dark: bool,
    /// The theme descends from an islands theme (`parentTheme` names
    /// `Islands Dark` or `Islands Light`): the roles the sources do not set
    /// come from the bundled islands theme, not from the original base.
    pub islands: bool,
    /// The look of the controls: classic for a UI theme that is not based
    /// on a New UI theme; `None` for a scheme alone (the base theme's).
    pub controls: Option<Controls>,
    /// UI theme and its available ancestors, nearest first.
    pub ui_layers: Vec<&'a UiThemeData>,
    /// Scheme and its available ancestors, nearest first.
    pub scheme_layers: Vec<&'a Scheme>,
    pub sources: Vec<String>,
    /// Notes collected while the chains were built.
    pub warnings: Vec<Warning>,
}

pub(crate) fn convert(inputs: Inputs<'_>) -> Imported {
    let base_kind = if inputs.dark {
        ThemeKind::Dark
    } else {
        ThemeKind::Light
    };
    let base = builtin::import_base(base_kind, inputs.islands);
    let mut palette = BTreeMap::new();
    for layer in inputs.ui_layers.iter().rev() {
        palette.extend(layer.palette.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let mut cx = Converter {
        kind: base_kind,
        islands: inputs.islands,
        controls: inputs.controls,
        ui_layers: inputs.ui_layers,
        chain: SchemeChain {
            layers: inputs.scheme_layers,
        },
        bundled: true,
        palette,
        report: ImportReport {
            name: inputs.name.clone(),
            kind: inputs.kind,
            sources: inputs.sources,
            base: builtin::import_id_for(base_kind, inputs.islands).to_owned(),
            warnings: inputs.warnings,
            ..ImportReport::default()
        },
        warned: HashSet::new(),
    };

    let mut theme = base.clone();
    theme.id = slug(&inputs.name);
    theme.name = inputs.name;
    theme.kind = base_kind;
    theme.author = inputs.author;
    if let Some(controls) = inputs.controls {
        theme.controls = controls;
    }
    theme.description = Some(
        match inputs.kind {
            SourceKind::UiTheme => "Imported PyCharm UI theme",
            SourceKind::Scheme => "Imported PyCharm colour scheme",
        }
        .to_owned(),
    );

    cx.fill_ui(&mut theme.ui, &base.ui, base_kind == ThemeKind::Dark);
    cx.note_text_from_background();
    cx.fill_text(&mut theme.text, &base.text);
    cx.fill_metrics(&mut theme);
    cx.fill_fonts(&mut theme);
    cx.collect_unmapped();
    cx.finish();
    Imported {
        theme,
        report: cx.report,
    }
}

/// The colour the reference's code uses for a role when no theme key sets
/// it (`JBColor.namedColor(key, default)`), in a dark or a light theme; it
/// stands in before a fallback role and the base theme.
fn code_default(role: UiRole, dark: bool) -> Option<Rgba> {
    match role {
        // `LineStatusMarkerPopupPanel.getBorderColor`:
        // `JBColor(Gray._206, Gray._75)`.
        UiRole::MarkerPopupBorder => Some(if dark {
            Rgba::rgb(75, 75, 75)
        } else {
            Rgba::rgb(206, 206, 206)
        }),
        // `Notification.foreground` of `darcula.theme.json` (the base of a
        // theme without a parent); in a light theme `NotificationsManagerImpl`
        // takes `DEFAULT_TEXT_COLOR`, black.
        UiRole::NotificationText => Some(if dark {
            Rgba::rgb(0xbb, 0xbb, 0xbb)
        } else {
            Rgba::rgb(0, 0, 0)
        }),
        // `NotificationsManagerImpl.BORDER_COLOR`:
        // `JBColor(0xCDB2B2B2, 0xCD565A5C)`, 80 % opaque.
        UiRole::NotificationBorder => Some(if dark {
            Rgba::new(0x56, 0x5a, 0x5c, 0xcd)
        } else {
            Rgba::new(0xb2, 0xb2, 0xb2, 0xcd)
        }),
        _ => None,
    }
}

struct Converter<'a> {
    kind: ThemeKind,
    /// The base theme is an islands theme (see [`Inputs::islands`]).
    islands: bool,
    controls: Option<Controls>,
    ui_layers: Vec<&'a UiThemeData>,
    chain: SchemeChain<'a>,
    /// Whether the bundled base schemes (the `parent_scheme` the inputs
    /// lack) answer lookups: for the text styles and the roles a scheme
    /// sets, not for the roles of the UI theme (the base theme keeps those).
    bundled: bool,
    palette: BTreeMap<String, String>,
    report: ImportReport,
    /// Keys already reported as invalid, to warn once.
    warned: HashSet<String>,
}

/// A colour found for a UI role.
struct Hit {
    color: Rgba,
    source: KeySource,
    key: String,
    /// Reached through the `*.property` form of the key.
    wildcard: bool,
}

#[derive(Clone, Copy)]
enum Part {
    Fg,
    Bg,
    Effect,
    Stripe,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Part::Fg => "FOREGROUND",
            Part::Bg => "BACKGROUND",
            Part::Effect => "EFFECT_COLOR",
            Part::Stripe => "ERROR_STRIPE_COLOR",
        }
    }
}

impl<'a> Converter<'a> {
    // ---- lookups ----------------------------------------------------

    /// Finds a `ui` key. Each layer is searched for the key and then for its
    /// wildcard form before the next ancestor is consulted, so a wildcard of
    /// a child theme beats a specific key of its parent.
    fn ui_lookup(&self, key: &str) -> Option<(&'a Value, String, bool)> {
        let wildcard = wildcard_property(key).map(|p| format!("*.{p}"));
        for layer in &self.ui_layers {
            for (is_wildcard, candidate) in
                std::iter::once((false, key)).chain(wildcard.as_deref().map(|w| (true, w)))
            {
                if let Some(value) = layer.ui.get(candidate) {
                    return Some((value, candidate.to_owned(), is_wildcard));
                }
                let platform_default = format!("{candidate}.os.default");
                if let Some(value) = layer.ui.get(&platform_default) {
                    return Some((value, platform_default, is_wildcard));
                }
            }
        }
        None
    }

    fn warn_invalid(&mut self, key: &str, value: &str) {
        if self.warned.insert(format!("{key}={value}")) {
            self.report.warn(
                Level::Warning,
                WarningKind::InvalidValue {
                    key: key.to_owned(),
                    value: value.to_owned(),
                },
                format!("`{key}` has the value `{value}`, which is not a colour"),
            );
        }
    }

    fn ui_color(&mut self, key: &str, value: &Value) -> Option<Rgba> {
        let Value::String(text) = value else {
            self.warn_invalid(key, &value.to_string());
            return None;
        };
        let mut current = text.as_str();
        for _ in 0..8 {
            match self.palette.get(current) {
                Some(next) if next != current => current = next.as_str(),
                _ => break,
            }
        }
        match Rgba::parse(current) {
            Ok(color) => Some(color),
            Err(_) => {
                let looks_like_name = !text.starts_with('#')
                    && text
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | ' '));
                if looks_like_name && current == text {
                    if self.warned.insert(format!("palette:{text}")) {
                        self.report.warn(
                            Level::Warning,
                            WarningKind::UnresolvedPaletteName {
                                key: key.to_owned(),
                                name: text.clone(),
                            },
                            format!(
                                "`{key}` refers to `{text}`, which no available theme defines in its colors (a parent theme may be missing)"
                            ),
                        );
                    }
                } else {
                    self.warn_invalid(key, text);
                }
                None
            }
        }
    }

    fn scheme_color(&mut self, key: &str, raw: &str) -> Option<Rgba> {
        match Rgba::parse_scheme(raw) {
            Ok(color) => Some(color),
            Err(_) => {
                self.warn_invalid(key, raw);
                None
            }
        }
    }

    fn eval_ui(&mut self, key: &str) -> Option<Hit> {
        let (value, effective, wildcard) = self.ui_lookup(key)?;
        let color = self.ui_color(&effective, value)?;
        Some(Hit {
            color,
            source: KeySource::UiTheme,
            key: effective,
            wildcard,
        })
    }

    fn eval_color(&mut self, key: &str) -> Option<Hit> {
        let raw = self.chain.color_with(key, self.bundled)?.to_owned();
        let color = self.scheme_color(key, &raw)?;
        Some(Hit {
            color,
            source: KeySource::SchemeColor,
            key: key.to_owned(),
            wildcard: false,
        })
    }

    fn eval_attr(&mut self, key: &str, part: Part) -> Option<Hit> {
        let attribute = self.chain.attribute_with(key, self.bundled)?;
        let from_option =
            matches!(part, Part::Bg) && attribute.background_option != BackgroundOption::Unused;
        let raw = match part {
            Part::Fg => attribute.foreground,
            Part::Bg => attribute.background,
            Part::Effect => attribute.effect_color,
            Part::Stripe => attribute.error_stripe,
        }?;
        // A background the `<colors>` option put there is reported under
        // that option, the key the file wrote.
        let (source, label) = if from_option {
            (KeySource::SchemeColor, BACKGROUND_OPTION.to_owned())
        } else {
            (KeySource::SchemeAttribute, format!("{key}.{}", part.name()))
        };
        let color = self.scheme_color(&label, &raw)?;
        Some(Hit {
            color,
            source,
            key: label,
            wildcard: false,
        })
    }

    fn eval(&mut self, src: Src) -> Option<Hit> {
        match src {
            Src::Ui(key) => self.eval_ui(key),
            Src::Color(key) => self.eval_color(key),
            Src::Key(key) => self.eval_ui(key).or_else(|| self.eval_color(key)),
            Src::Fg(key) => self.eval_attr(key, Part::Fg),
            Src::Bg(key) => self.eval_attr(key, Part::Bg),
            Src::Effect(key) => self.eval_attr(key, Part::Effect),
            Src::Stripe(key) => self.eval_attr(key, Part::Stripe),
            Src::Palette(key) => self.eval_palette(key),
        }
    }

    /// Finds a key of the icon colour palette: in each layer, nearest
    /// first, the key itself, in a dark theme the key with `.Dark`, then a
    /// `#RRGGBB` key naming the colour the key stands for in the icons.
    fn eval_palette(&mut self, key: &str) -> Option<Hit> {
        let dark = self.kind == ThemeKind::Dark;
        let dark_key = format!("{key}.Dark");
        let stands_for = reference_color(key, self.kind);
        let mut found: Option<(String, String)> = None;
        for layer in &self.ui_layers {
            let palette = &layer.icon_palette;
            let hit = palette
                .get_key_value(key)
                .or_else(|| {
                    dark.then(|| palette.get_key_value(dark_key.as_str()))
                        .flatten()
                })
                .or_else(|| {
                    let color = stands_for?;
                    palette
                        .iter()
                        .find(|(k, _)| k.starts_with('#') && Rgba::parse(k).ok() == Some(color))
                });
            if let Some((k, v)) = hit {
                found = Some((k.clone(), v.clone()));
                break;
            }
        }
        let (found_key, value) = found?;
        let label = format!("ColorPalette.{found_key}");
        let color = self.ui_color(&label, &Value::String(value))?;
        Some(Hit {
            color,
            source: KeySource::IconPalette,
            key: found_key,
            wildcard: false,
        })
    }

    // ---- UI colours -------------------------------------------------

    fn fill_ui(&mut self, out: &mut UiColors, base: &UiColors, dark: bool) {
        let mut direct: Vec<Option<Rgba>> = vec![None; UiRole::COUNT];
        // Roles that a colour scheme sets, as opposed to the UI theme.
        let mut from_scheme = vec![false; UiRole::COUNT];
        for &role in UiRole::ALL {
            let target = format!("ui.{role}");
            // A key the theme sets for this very component beats a wildcard
            // reached through an earlier source.
            let mut hits: Vec<Hit> = Vec::new();
            self.bundled = mapping::is_scheme_role(role);
            for &src in ui_sources(role) {
                let Some(hit) = self.eval(src) else {
                    continue;
                };
                // Several sources of a role can end at the same wildcard key.
                if hits
                    .iter()
                    .any(|h| h.source == hit.source && h.key == hit.key)
                {
                    continue;
                }
                hits.push(hit);
            }
            let winner = hits
                .iter()
                .position(|h| !h.wildcard)
                .or(if hits.is_empty() { None } else { Some(0) });
            for (i, hit) in hits.into_iter().enumerate() {
                let used = Some(i) == winner;
                if used {
                    direct[role.index()] = Some(hit.color);
                    from_scheme[role.index()] = matches!(
                        hit.source,
                        KeySource::SchemeColor | KeySource::SchemeAttribute
                    );
                }
                self.report.mapped.push(MappedKey {
                    source: hit.source,
                    key: hit.key,
                    target: target.clone(),
                    used,
                });
            }
        }
        self.bundled = true;
        for &role in UiRole::ALL {
            let target = format!("ui.{role}");
            if let Some(color) = direct[role.index()] {
                out[role] = color;
                continue;
            }
            // A theme under an islands theme: the parent chain that the
            // inputs lack sets the other roles, and the bundled islands
            // theme holds their values; a stand-in or a colour derived from
            // a UI theme role would only be a guess. What the editor scheme
            // sets still drives the roles that follow it (the gutter
            // follows the editor background).
            if self.islands {
                let mut ancestor = role.fallback();
                let mut derived = None;
                while let Some(from) = ancestor {
                    if from_scheme[from.index()]
                        && let Some(color) = direct[from.index()]
                    {
                        derived = Some((from, color));
                        break;
                    }
                    ancestor = from.fallback();
                }
                match derived {
                    Some((from, color)) => {
                        out[role] = color;
                        self.report.approximations.push(Approximation {
                            key: format!("ui.{from}"),
                            target,
                            note: format!("no key sets it; takes the colour of ui.{from}"),
                        });
                    }
                    None => {
                        out[role] = base[role];
                        self.report.defaulted.push(Defaulted { target });
                    }
                }
                continue;
            }
            if let Some(color) = code_default(role, dark) {
                out[role] = color;
                self.report.approximations.push(Approximation {
                    key: format!("ui.{role}"),
                    target,
                    note: "no key sets it; takes the reference's default colour".to_owned(),
                });
                continue;
            }
            let mut ancestor = role.fallback();
            let mut derived = None;
            while let Some(from) = ancestor {
                if let Some(color) = direct[from.index()] {
                    derived = Some((from, color));
                    break;
                }
                ancestor = from.fallback();
            }
            let control = self
                .controls
                .and_then(|controls| control_default(role, controls, self.kind));
            match (derived, control) {
                (Some((from, color)), _) => {
                    out[role] = color;
                    self.report.approximations.push(Approximation {
                        key: format!("ui.{from}"),
                        target,
                        note: format!("no key sets it; takes the colour of ui.{from}"),
                    });
                }
                // Check boxes and tree arrows look as the reference draws
                // them for the theme's kind of controls.
                (None, Some(color)) => {
                    out[role] = color;
                    let look = match self.controls {
                        Some(Controls::Classic) => "classic",
                        _ => "New UI",
                    };
                    self.report.approximations.push(Approximation {
                        key: target.clone(),
                        target,
                        note: format!("no key sets it; the colour of the {look} controls"),
                    });
                }
                (None, None) => {
                    out[role] = base[role];
                    self.report.defaulted.push(Defaulted { target });
                }
            }
        }
    }

    /// The file sets no text colour when `TEXT` was made from the `<colors>`
    /// option `BACKGROUND` alone; the black text that follows is worth a note.
    fn note_text_from_background(&mut self) {
        let Some(layer) = self
            .chain
            .layers
            .iter()
            .find(|l| l.attributes.contains_key(TEXT))
        else {
            return;
        };
        if layer.attributes[TEXT].background_option != BackgroundOption::Created {
            return;
        }
        let scheme = layer.name.clone();
        self.report.warn(
            Level::Info,
            WarningKind::TextFromBackground {
                scheme: scheme.clone(),
            },
            format!(
                "`{scheme}` sets the editor background with the deprecated `BACKGROUND` colour and has no `TEXT` attribute; as in PyCharm, the editor text is black"
            ),
        );
    }

    // ---- text styles ------------------------------------------------

    fn style_of(&mut self, key: &str, attribute: &Attribute, target: &str) -> TextStyle {
        let mut style = TextStyle::default();
        let parse = |cx: &mut Self, part: Part, raw: &Option<String>| {
            raw.as_deref()
                .and_then(|r| cx.scheme_color(&format!("{key}.{}", part.name()), r))
        };
        style.fg = parse(self, Part::Fg, &attribute.foreground);
        style.bg = parse(self, Part::Bg, &attribute.background);
        let effect_color = parse(self, Part::Effect, &attribute.effect_color);
        if let Some(raw) = &attribute.font_type {
            match raw.parse::<u32>() {
                Ok(bits) => {
                    style.bold = (bits & 1 != 0).then_some(true);
                    style.italic = (bits & 2 != 0).then_some(true);
                }
                Err(_) => self.warn_invalid(&format!("{key}.FONT_TYPE"), raw),
            }
        }
        let note = |cx: &mut Self, text: String| {
            cx.report.approximations.push(Approximation {
                key: key.to_owned(),
                target: target.to_owned(),
                note: text,
            });
        };
        match attribute.effect_type.as_deref().map(str::parse::<u32>) {
            None => {
                if effect_color.is_some() {
                    note(
                        self,
                        "EFFECT_COLOR without EFFECT_TYPE is ignored".to_owned(),
                    );
                }
            }
            // The reference paints an effect only when both its type and its
            // colour are set (`AttributesFlyweight.getAllEffects`), so a type
            // without a colour draws nothing rather than a line in the text
            // colour.
            Some(Ok(_)) if effect_color.is_none() => {}
            Some(Ok(kind)) => {
                let underline = |style: &mut TextStyle, kind: UnderlineStyle| {
                    style.underline = Some(kind);
                    style.underline_color = effect_color;
                };
                match kind {
                    0 => style.border = effect_color,
                    1 => underline(&mut style, UnderlineStyle::Single),
                    2 => underline(&mut style, UnderlineStyle::Wavy),
                    3 => {
                        style.strikethrough = Some(true);
                        if effect_color != style.fg {
                            note(
                                self,
                                "the strikeout colour follows the text colour".to_owned(),
                            );
                        }
                    }
                    4 => underline(&mut style, UnderlineStyle::Thick),
                    5 => underline(&mut style, UnderlineStyle::Dotted),
                    other => note(
                        self,
                        format!("EFFECT_TYPE {other} is not supported and is ignored"),
                    ),
                }
            }
            Some(Err(_)) => {
                let raw = attribute.effect_type.clone().unwrap_or_default();
                self.warn_invalid(&format!("{key}.EFFECT_TYPE"), &raw);
            }
        }
        style
    }

    fn fill_text(&mut self, out: &mut TextStyles, base: &TextStyles) {
        let mut direct: Vec<Option<TextStyle>> = vec![None; TextRole::COUNT];
        for &role in TextRole::ALL {
            let target = format!("syntax.{role}");
            let mut found: Vec<(&str, TextStyle)> = Vec::new();
            for &key in text_sources(role) {
                if let Some(attribute) = self.chain.attribute(key) {
                    let style = self.style_of(key, &attribute, &target);
                    found.push((key, style));
                }
            }
            for (i, (key, style)) in found.iter().enumerate() {
                self.report.mapped.push(MappedKey {
                    source: KeySource::SchemeAttribute,
                    key: (*key).to_owned(),
                    target: target.clone(),
                    used: i == 0,
                });
                if i > 0 && *style != found[0].1 {
                    self.report.approximations.push(Approximation {
                        key: (*key).to_owned(),
                        target: target.clone(),
                        note: format!(
                            "also defined with a different style; {} is used for the whole token",
                            found[0].0
                        ),
                    });
                }
            }
            direct[role.index()] = found.into_iter().next().map(|(_, style)| style);
        }
        for &role in TextRole::ALL {
            let target = format!("syntax.{role}");
            if let Some(style) = &direct[role.index()] {
                out[role] = style.clone();
                continue;
            }
            let mut ancestor = role.fallback();
            let mut derived = None;
            while let Some(from) = ancestor {
                if let Some(style) = &direct[from.index()] {
                    derived = Some((from, style.clone()));
                    break;
                }
                ancestor = from.fallback();
            }
            match derived {
                Some((from, style)) => {
                    out[role] = style;
                    self.report.approximations.push(Approximation {
                        key: format!("syntax.{from}"),
                        target,
                        note: format!("no key sets it; takes the style of syntax.{from}"),
                    });
                }
                None => {
                    out[role] = base[role].clone();
                    self.report.defaulted.push(Defaulted { target });
                }
            }
        }
        // The Python language services draw a hint (LSP severity Hint, the
        // information level) as a grey dotted line
        // (`PyLspToolIntegrationProvider.getEnforcedTextAttributes`,
        // IntelliJ Community, Apache-2.0), where the schemes leave the
        // information attributes empty.
        let hint = &mut out[TextRole::DiagnosticHint];
        if hint.is_plain() {
            *hint = TextStyle {
                underline: Some(UnderlineStyle::Dotted),
                underline_color: Some(Rgba::rgb(0x80, 0x80, 0x80)),
                ..TextStyle::default()
            };
        }
    }

    // ---- metrics and fonts ------------------------------------------

    fn fill_metrics(&mut self, theme: &mut Theme) {
        for &metric in Metric::ALL {
            let (keys, scale) = metric_sources(metric);
            let target = format!("metrics.{metric}");
            let mut taken = false;
            for &key in keys {
                let Some((value, effective, _)) = self.ui_lookup(key) else {
                    continue;
                };
                let number = match value {
                    Value::Number(n) => n.as_f64(),
                    Value::String(s) => s.trim().parse().ok(),
                    _ => None,
                };
                let Some(number) = number else {
                    self.warn_invalid(&effective, &value.to_string());
                    continue;
                };
                self.report.mapped.push(MappedKey {
                    source: KeySource::UiTheme,
                    key: effective,
                    target: target.clone(),
                    used: !taken,
                });
                if !taken {
                    theme.metrics[metric] = number as f32 * scale;
                    taken = true;
                }
            }
            if !taken {
                self.report.defaulted.push(Defaulted { target });
            }
        }
    }

    fn fill_fonts(&mut self, theme: &mut Theme) {
        let note = |cx: &mut Self, key: &str, target: &str| {
            cx.report.mapped.push(MappedKey {
                source: KeySource::SchemeOption,
                key: key.to_owned(),
                target: format!("font.{target}"),
                used: true,
            });
        };
        let layers: Vec<&Scheme> = self
            .chain
            .layers
            .iter()
            .filter(|l| !l.bundled)
            .copied()
            .collect();
        let first = |pick: &dyn Fn(&Scheme) -> bool| layers.iter().find(|s| pick(s)).copied();
        if let Some(s) = first(&|s| s.fonts.editor_name.is_some())
            && let Some(name) = &s.fonts.editor_name
        {
            let mut family = vec![name.clone()];
            family.extend(
                theme
                    .fonts
                    .editor_family
                    .iter()
                    .filter(|f| *f != name)
                    .cloned(),
            );
            theme.fonts.editor_family = family;
            note(self, "EDITOR_FONT_NAME", "editor-family");
        }
        if let Some(s) = first(&|s| s.fonts.editor_size.is_some())
            && let Some(size) = s.fonts.editor_size
        {
            theme.fonts.editor_size = size;
            note(self, "EDITOR_FONT_SIZE", "editor-size");
        }
        if let Some(s) = first(&|s| s.fonts.line_spacing.is_some())
            && let Some(spacing) = s.fonts.line_spacing
        {
            theme.fonts.editor_line_spacing = spacing;
            note(self, "LINE_SPACING", "editor-line-spacing");
        }
        if let Some(s) = first(&|s| s.fonts.ligatures.is_some())
            && let Some(ligatures) = s.fonts.ligatures
        {
            theme.fonts.editor_ligatures = ligatures;
            note(self, "EDITOR_LIGATURES", "editor-ligatures");
        }
        if let Some(s) = first(&|s| s.fonts.console_name.is_some())
            && let Some(name) = &s.fonts.console_name
        {
            let mut family = vec![name.clone()];
            family.extend(
                theme
                    .fonts
                    .terminal_family
                    .iter()
                    .filter(|f| *f != name)
                    .cloned(),
            );
            theme.fonts.terminal_family = family;
            note(self, "console font name", "terminal-family");
        }
        if let Some(s) = first(&|s| s.fonts.console_size.is_some())
            && let Some(size) = s.fonts.console_size
        {
            theme.fonts.terminal_size = size;
            note(self, "console font size", "terminal-size");
        }
    }

    // ---- unmapped ---------------------------------------------------

    fn collect_unmapped(&mut self) {
        let known = known_keys();
        let mut counts: BTreeMap<(KeySource, String), usize> = BTreeMap::new();
        for layer in &self.ui_layers {
            for key in layer.ui.keys() {
                if !is_known_ui_key(key, known) {
                    *counts.entry((KeySource::UiTheme, key.clone())).or_default() += 1;
                }
            }
            for key in layer.icon_palette.keys() {
                if !is_known_palette_key(key, known) {
                    *counts
                        .entry((KeySource::IconPalette, key.clone()))
                        .or_default() += 1;
                }
            }
            for (section, leaves) in &layer.other {
                *counts
                    .entry((KeySource::UiThemeSection, section.clone()))
                    .or_default() += leaves;
            }
        }
        for layer in self.chain.layers.iter().filter(|l| !l.bundled) {
            for key in layer.colors.keys() {
                if !known.colors.contains(key.as_str()) {
                    *counts
                        .entry((KeySource::SchemeColor, key.clone()))
                        .or_default() += 1;
                }
            }
            for key in layer.attributes.keys() {
                if !known.attributes.contains(key.as_str()) {
                    *counts
                        .entry((KeySource::SchemeAttribute, key.clone()))
                        .or_default() += 1;
                }
            }
            for key in &layer.other_options {
                *counts
                    .entry((KeySource::SchemeOption, key.clone()))
                    .or_default() += 1;
            }
        }
        self.report.unmapped = counts
            .into_iter()
            .map(|((source, key), count)| UnmappedKey { source, key, count })
            .collect();
    }

    fn finish(&mut self) {
        let missing = self.report.defaulted.len();
        if missing > 0 {
            let base = self.report.base.clone();
            self.report.warn(
                Level::Info,
                WarningKind::MissingBaseValues { count: missing },
                format!("{missing} tokens have no value in the sources; {base} supplies them"),
            );
        }
    }
}

/// A palette key some role reads: one of the table, with `.Dark`, or a
/// `#RRGGBB` key (it may name the colour of a key of the table).
fn is_known_palette_key(key: &str, known: &mapping::KnownKeys) -> bool {
    let key = key.strip_suffix(".Dark").unwrap_or(key);
    known.palette.contains(key) || key.starts_with('#')
}

fn is_known_ui_key(key: &str, known: &mapping::KnownKeys) -> bool {
    if known.ui.contains(key) {
        return true;
    }
    // Platform specific variants: `*.selectionBackground.os.windows`.
    let key = key.split_once(".os.").map_or(key, |(head, _)| head);
    if known.ui.contains(key) {
        return true;
    }
    key.strip_prefix("*.")
        .is_some_and(|property| known.wildcard_properties.contains(property))
}

/// `Dracula Colorful` becomes `dracula-colorful`.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "imported".to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Dracula Colorful"), "dracula-colorful");
        assert_eq!(
            slug("Monokai Pro (Filter Octagon)"),
            "monokai-pro-filter-octagon"
        );
        assert_eq!(slug("  "), "imported");
        assert_eq!(slug("A--B"), "a-b");
    }
}
