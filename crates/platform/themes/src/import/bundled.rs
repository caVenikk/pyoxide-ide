//! The base colour schemes of the IDE, `Default` and `Darcula`, with the
//! attributes the Python plugin adds to them. A scheme names one of them as
//! its `parent_scheme` and the theme files never contain it; the values a
//! scheme leaves unset come from it. The files are those of the IntelliJ
//! Platform and the Python plugin of the reference (Apache-2.0, see
//! `notices/themes-scheme-parent.md`) and are used only as data.

use std::sync::OnceLock;

use super::scheme::Scheme;

const DEFAULTS: &str = include_str!("../../schemes/DefaultColorSchemesManager.xml");
const PYTHON_DARCULA: &str = include_str!("../../schemes/PythonDarcula.xml");
const PYTHON_DEFAULT: &str = include_str!("../../schemes/PythonDefault.xml");

fn all() -> &'static [Scheme] {
    static ALL: OnceLock<Vec<Scheme>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut schemes = Vec::new();
        for (text, entry) in [
            (DEFAULTS, "DefaultColorSchemesManager.xml"),
            (PYTHON_DARCULA, "PythonDarcula.xml"),
            (PYTHON_DEFAULT, "PythonDefault.xml"),
        ] {
            let origin = format!("(bundled) {entry}");
            let parsed = Scheme::parse(text, &origin, entry).expect("bundled scheme parses");
            schemes.extend(parsed.into_iter().map(|mut s| {
                s.bundled = true;
                s
            }));
        }
        schemes
    })
}

/// The bundled scheme `name` (`Default` or `Darcula`), then the attributes
/// the bundled language files add to it, then its parents the same way.
/// Empty for any other name.
pub(crate) fn chain(name: &str) -> Vec<&'static Scheme> {
    let mut layers: Vec<&'static Scheme> = Vec::new();
    let mut next = Some(name.to_owned());
    while let Some(name) = next.take() {
        let Some(scheme) = all().iter().find(|s| !s.additional && s.name == name) else {
            break;
        };
        if layers.iter().any(|l| std::ptr::eq(*l, scheme)) {
            break;
        }
        layers.push(scheme);
        layers.extend(all().iter().filter(|s| s.additional && s.name == name));
        next = scheme.parent.clone();
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn darcula_is_followed_by_its_python_attributes_and_the_default_scheme() {
        let layers = chain("Darcula");
        let names: Vec<_> = layers
            .iter()
            .map(|s| (s.name.as_str(), s.additional))
            .collect();
        assert_eq!(
            names,
            [
                ("Darcula", false),
                ("Darcula", true),
                ("Default", false),
                ("Default", true)
            ]
        );
        assert!(layers.iter().all(|s| s.bundled));
        assert_eq!(
            layers[0].attributes["DEBUGGER_INLINED_VALUES_MODIFIED"]
                .foreground
                .as_deref(),
            Some("ca7e03")
        );
    }

    #[test]
    fn only_the_two_base_schemes_are_bundled() {
        assert_eq!(chain("Default").len(), 2);
        assert!(chain("Dracula").is_empty());
    }
}
