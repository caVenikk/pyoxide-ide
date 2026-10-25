//! Which inspections run and how severe their problems are: a setting
//! (`[sql.inspections]` of the settings file the product keeps), edited on
//! the Inspections page and by the *Disable inspection* fix.

use std::collections::BTreeMap;
use std::fmt;

use code_intel::Severity;
use serde::{Deserialize, Serialize};

/// An inspection of the first set (ADR 0022). The ids are the reference's
/// public inspection ids, so `-- noinspection` comments written in either
/// application mean the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Inspection {
    /// A syntax error: always an error, never suppressed (as in the
    /// reference, where syntax errors are no inspection).
    Syntax,
    /// A table, view, schema, column or routine that does not resolve.
    Resolve,
    /// An unqualified column found in more than one source of `FROM`.
    AmbiguousColumn,
    /// `DELETE` or `UPDATE` without `WHERE`.
    WithoutWhere,
    /// An `INSERT` row with a different number of values than columns.
    InsertValues,
    /// A comparison with `NULL` by `=`, `<>` or `!=`, which is never true.
    /// The reference reports it with its constant expressions, so it shares
    /// that id.
    NullComparison,
    /// A common table expression nobody references.
    UnusedCte,
}

impl Inspection {
    pub const ALL: [Inspection; 7] = [
        Inspection::Syntax,
        Inspection::Resolve,
        Inspection::AmbiguousColumn,
        Inspection::WithoutWhere,
        Inspection::InsertValues,
        Inspection::NullComparison,
        Inspection::UnusedCte,
    ];

    /// The id: the code of its diagnostics, the key of its setting and the
    /// word of a `-- noinspection` comment.
    pub fn id(self) -> &'static str {
        match self {
            Inspection::Syntax => "SqlSyntax",
            Inspection::Resolve => "SqlResolve",
            Inspection::AmbiguousColumn => "SqlAmbiguousColumn",
            Inspection::WithoutWhere => "SqlWithoutWhere",
            Inspection::InsertValues => "SqlInsertValues",
            Inspection::NullComparison => "SqlConstantExpression",
            Inspection::UnusedCte => "SqlUnusedCte",
        }
    }

    pub fn from_id(id: &str) -> Option<Inspection> {
        Inspection::ALL.into_iter().find(|i| i.id() == id)
    }

    /// The name on the Inspections page and in *Disable inspection*.
    pub fn title(self) -> &'static str {
        match self {
            Inspection::Syntax => "Syntax errors",
            Inspection::Resolve => "Unresolved reference",
            Inspection::AmbiguousColumn => "Ambiguous column reference",
            Inspection::WithoutWhere => "DELETE or UPDATE without WHERE",
            Inspection::InsertValues => "INSERT values count",
            Inspection::NullComparison => "Comparison with NULL",
            Inspection::UnusedCte => "Unused common table expression",
        }
    }

    /// The default level. Our choice from public documentation; the
    /// reference's defaults need a screenshot of its Inspections page.
    pub fn default_level(self) -> InspectionLevel {
        match self {
            Inspection::Syntax | Inspection::Resolve | Inspection::InsertValues => {
                InspectionLevel::Error
            }
            Inspection::AmbiguousColumn | Inspection::WithoutWhere | Inspection::NullComparison => {
                InspectionLevel::Warning
            }
            Inspection::UnusedCte => InspectionLevel::WeakWarning,
        }
    }

    /// Problems can be suppressed by a comment and the inspection turned
    /// off.
    pub fn suppressible(self) -> bool {
        self != Inspection::Syntax
    }
}

impl fmt::Display for Inspection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// How an inspection reports: not at all, or at a severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionLevel {
    Off,
    Error,
    Warning,
    WeakWarning,
    Information,
}

impl InspectionLevel {
    pub fn severity(self) -> Option<Severity> {
        match self {
            InspectionLevel::Off => None,
            InspectionLevel::Error => Some(Severity::Error),
            InspectionLevel::Warning => Some(Severity::Warning),
            InspectionLevel::WeakWarning => Some(Severity::WeakWarning),
            InspectionLevel::Information => Some(Severity::Information),
        }
    }
}

/// The levels changed from their defaults, by inspection id:
///
/// ```toml
/// [sql.inspections]
/// SqlWithoutWhere = "error"
/// SqlUnusedCte = "off"
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InspectionProfile {
    levels: BTreeMap<String, InspectionLevel>,
}

impl InspectionProfile {
    pub fn level(&self, inspection: Inspection) -> InspectionLevel {
        if !inspection.suppressible() {
            return inspection.default_level();
        }
        self.levels
            .get(inspection.id())
            .copied()
            .unwrap_or_else(|| inspection.default_level())
    }

    pub fn set(&mut self, inspection: Inspection, level: InspectionLevel) {
        if !inspection.suppressible() {
            return;
        }
        if level == inspection.default_level() {
            self.levels.remove(inspection.id());
        } else {
            self.levels.insert(inspection.id().to_string(), level);
        }
    }

    pub fn disable(&mut self, inspection: Inspection) {
        self.set(inspection, InspectionLevel::Off);
    }

    pub fn is_enabled(&self, inspection: Inspection) -> bool {
        self.level(inspection) != InspectionLevel::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_and_settings() {
        let mut p = InspectionProfile::default();
        assert_eq!(p.level(Inspection::Resolve), InspectionLevel::Error);
        p.disable(Inspection::UnusedCte);
        p.set(Inspection::WithoutWhere, InspectionLevel::Error);
        p.set(Inspection::Resolve, InspectionLevel::Error);
        p.disable(Inspection::Syntax);
        assert!(p.is_enabled(Inspection::Syntax));
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"SqlUnusedCte": "off", "SqlWithoutWhere": "error"})
        );
        let back: InspectionProfile = serde_json::from_value(json).unwrap();
        assert_eq!(back.level(Inspection::UnusedCte), InspectionLevel::Off);
        assert_eq!(Inspection::from_id("SqlResolve"), Some(Inspection::Resolve));
        assert_eq!(
            InspectionLevel::WeakWarning.severity(),
            Some(Severity::WeakWarning)
        );
    }
}
