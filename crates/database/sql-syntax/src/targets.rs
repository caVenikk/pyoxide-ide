use crate::classify::{DdlAction, DdlTarget, Ident, ObjectKind, QualifiedName};
use crate::dialect::Dialect;
use crate::lexer::TokenKind;
use crate::tree::{SyntaxKind, SyntaxNode};

/// The DDL target of a statement node, read from the tree: the same answer as
/// the token scan of [`crate::classify`], without guessing where names end.
pub(crate) fn ddl_target(stmt: SyntaxNode<'_>, dialect: Dialect) -> Option<DdlTarget> {
    use SyntaxKind as K;
    let (action, object) = match stmt.kind() {
        K::CreateTable => (DdlAction::Create, ObjectKind::Table),
        K::CreateView => (DdlAction::Create, ObjectKind::View),
        K::CreateMaterializedView => (DdlAction::Create, ObjectKind::MaterializedView),
        K::CreateIndex => (DdlAction::Create, ObjectKind::Index),
        K::CreateSequence => (DdlAction::Create, ObjectKind::Sequence),
        K::CreateSchema => (DdlAction::Create, ObjectKind::Schema),
        K::CreateType | K::CreateTypeBody => (DdlAction::Create, ObjectKind::Type),
        K::CreateTrigger => (DdlAction::Create, ObjectKind::Trigger),
        K::CreateFunction => (DdlAction::Create, ObjectKind::Function),
        K::CreateProcedure => (DdlAction::Create, ObjectKind::Procedure),
        K::CreatePackage => (DdlAction::Create, ObjectKind::Package),
        K::CreatePackageBody => (DdlAction::Create, ObjectKind::PackageBody),
        K::AlterTable => (DdlAction::Alter, ObjectKind::Table),
        K::AlterObject => (DdlAction::Alter, object_of_words(&words(stmt), 1)?),
        K::DropStatement => (DdlAction::Drop, object_of_words(&words(stmt), 1)?),
        K::Truncate => (DdlAction::Truncate, ObjectKind::Table),
        K::CommentStatement => (DdlAction::Comment, object_of_words(&words(stmt), 2)?),
        _ => return None,
    };
    let refs: Vec<SyntaxNode<'_>> = stmt
        .child_nodes()
        .filter(|n| is_target_ref(n.kind()))
        .collect();
    let table_like = matches!(
        object,
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
    );
    let mut names = Vec::new();
    let mut on = None;
    match (action, object) {
        (DdlAction::Create, ObjectKind::Index | ObjectKind::Trigger) => {
            for r in &refs {
                match r.kind() {
                    K::TableRef => on = qualified(*r, dialect),
                    _ => names.extend(qualified(*r, dialect)),
                }
            }
        }
        (DdlAction::Drop, _) if !table_like => {
            // `DROP TRIGGER t ON tbl`: the table is not a target.
            for r in refs.iter().filter(|r| r.kind() != K::TableRef) {
                names.extend(qualified(*r, dialect));
            }
        }
        _ => {
            for r in &refs {
                names.extend(qualified(*r, dialect));
            }
        }
    }
    if names.is_empty() && on.is_none() {
        return None;
    }
    Some(DdlTarget {
        action,
        object,
        names,
        on,
    })
}

fn is_target_ref(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::TableRef
            | SyntaxKind::ObjectRef
            | SyntaxKind::FunctionRef
            | SyntaxKind::TypeRef
            | SyntaxKind::SchemaRef
            | SyntaxKind::ColumnRef
    )
}

/// The upper-case words among the direct tokens of the node.
fn words(stmt: SyntaxNode<'_>) -> Vec<String> {
    stmt.child_tokens()
        .filter(|t| t.kind() == TokenKind::Word)
        .map(|t| t.text().to_ascii_uppercase())
        .collect()
}

/// The object keyword among `words`, after the statement verb (and `ON` for
/// `COMMENT`), skipping modifiers.
fn object_of_words(words: &[String], skip: usize) -> Option<ObjectKind> {
    let mut materialized = false;
    for (n, w) in words.iter().enumerate().skip(skip) {
        let kind = match w.as_str() {
            "TABLE" => ObjectKind::Table,
            "VIEW" if materialized => ObjectKind::MaterializedView,
            "VIEW" => ObjectKind::View,
            "INDEX" => ObjectKind::Index,
            "SEQUENCE" => ObjectKind::Sequence,
            "SCHEMA" => ObjectKind::Schema,
            "DATABASE" => ObjectKind::Database,
            "FUNCTION" => ObjectKind::Function,
            "PROCEDURE" => ObjectKind::Procedure,
            "TRIGGER" => ObjectKind::Trigger,
            "TYPE" => ObjectKind::Type,
            "PACKAGE" if words.get(n + 1).is_some_and(|b| b == "BODY") => ObjectKind::PackageBody,
            "PACKAGE" => ObjectKind::Package,
            "SYNONYM" => ObjectKind::Synonym,
            "EXTENSION" => ObjectKind::Extension,
            "COLUMN" => ObjectKind::Column,
            "MATERIALIZED" => {
                materialized = true;
                continue;
            }
            "OR" | "REPLACE" | "TEMP" | "TEMPORARY" | "GLOBAL" | "LOCAL" | "UNLOGGED"
            | "UNIQUE" | "BITMAP" | "EDITIONABLE" | "NONEDITIONABLE" | "FORCE" | "NOFORCE"
            | "PUBLIC" | "CONSTRAINT" | "VIRTUAL" | "RECURSIVE" => continue,
            _ => return None,
        };
        return Some(kind);
    }
    None
}

/// The name of a reference node, part by part.
fn qualified(node: SyntaxNode<'_>, dialect: Dialect) -> Option<QualifiedName> {
    let parts: Vec<Ident> = node
        .tokens()
        .filter_map(|t| match t.kind() {
            TokenKind::Word => Some(Ident {
                text: t.text().to_string(),
                quoted: false,
            }),
            TokenKind::QuotedIdent => Some(Ident {
                text: dialect
                    .unquote(t.text())
                    .unwrap_or_else(|| t.text().to_string()),
                quoted: true,
            }),
            _ => None,
        })
        .collect();
    (!parts.is_empty()).then_some(QualifiedName { parts })
}
