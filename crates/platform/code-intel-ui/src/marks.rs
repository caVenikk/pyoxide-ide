//! Diagnostics of the services as the editor's own marks.

use std::collections::BTreeMap;
use std::ops::Range;

use code_intel::{Diagnostic, Severity};
use editor::{DiagnosticMark, MarkMessage, MarkSeverity};

use crate::document::Versions;

/// The marks of a document's diagnostics, reported in the text of
/// `version`: one mark per range, with a message per diagnostic (the
/// inspection widget counts the messages, as the reference counts every
/// problem); its severity is the most severe of them, and it is greyed or
/// struck out when one of them says so. `None` when the version is no
/// longer known (a newer report is on its way).
pub fn marks(
    versions: &Versions,
    version: u64,
    items: &[Diagnostic],
) -> Option<Vec<DiagnosticMark>> {
    let mut by_range: BTreeMap<(usize, usize), Vec<&Diagnostic>> = BTreeMap::new();
    for item in items {
        let range = versions.range(version, item.range.clone())?;
        by_range
            .entry((range.start, range.end))
            .or_default()
            .push(item);
    }
    Some(
        by_range
            .into_iter()
            .map(|((start, end), items)| mark(start..end, items))
            .collect(),
    )
}

fn mark(range: Range<usize>, mut items: Vec<&Diagnostic>) -> DiagnosticMark {
    // The most severe first (stable: servers keep their order).
    items.sort_by_key(|d| d.severity);
    DiagnosticMark {
        range,
        severity: severity(items[0].severity),
        unnecessary: items.iter().any(|d| d.unnecessary),
        deprecated: items.iter().any(|d| d.deprecated),
        messages: items
            .iter()
            .map(|d| MarkMessage {
                text: d.message.clone().into(),
                source: d.source.clone().map(Into::into),
                code: d.code.clone().map(Into::into),
                href: d.code_href.clone().map(Into::into),
            })
            .collect(),
    }
}

fn severity(severity: Severity) -> MarkSeverity {
    match severity {
        Severity::Error => MarkSeverity::Error,
        Severity::Warning => MarkSeverity::Warning,
        Severity::WeakWarning => MarkSeverity::WeakWarning,
        Severity::Information => MarkSeverity::Information,
    }
}

#[cfg(test)]
mod tests {
    use text::{Point, Rope};

    use super::*;

    fn diagnostic(
        range: Range<Point>,
        severity: Severity,
        message: &str,
        source: &str,
    ) -> Diagnostic {
        let mut d = Diagnostic::new(range, severity, message);
        d.source = Some(source.into());
        d
    }

    #[test]
    fn diagnostics_of_one_range_become_one_mark() {
        let versions = Versions::new(Rope::from_str("import os\nundefined()\n"));
        let mut unused = diagnostic(
            Point::new(0, 7)..Point::new(0, 9),
            Severity::Warning,
            "`os` imported but unused",
            "Ruff",
        );
        unused.code = Some("unused-import".into());
        unused.code_href = Some("https://docs.astral.sh/ruff/rules/unused-import".into());
        unused.unnecessary = true;
        let name = Point::new(1, 0)..Point::new(1, 9);
        let items = vec![
            unused,
            diagnostic(name.clone(), Severity::Warning, "Undefined name", "Ruff"),
            diagnostic(name, Severity::Error, "Unresolved reference", "ty"),
        ];
        let marks = marks(&versions, 1, &items).unwrap();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].range, 7..9);
        assert_eq!(marks[0].severity, MarkSeverity::Warning);
        assert!(marks[0].unnecessary && !marks[0].deprecated);
        assert_eq!(
            marks[0].messages[0].code.as_ref().unwrap().as_ref(),
            "unused-import"
        );
        assert!(marks[0].messages[0].href.is_some());
        // Two servers, one range: one mark, both messages, the error first.
        assert_eq!(marks[1].range, 10..19);
        assert_eq!(marks[1].severity, MarkSeverity::Error);
        let texts: Vec<_> = marks[1].messages.iter().map(|m| m.text.as_ref()).collect();
        assert_eq!(texts, ["Unresolved reference", "Undefined name"]);
        assert_eq!(marks[1].messages[0].source.as_ref().unwrap().as_ref(), "ty");
        // A version that is not known gives nothing.
        assert!(super::marks(&versions, 5, &items).is_none());
        assert_eq!(super::marks(&versions, 5, &[]), Some(Vec::new()));
    }
}
