//! File Structure (Cmd+F12): the symbols of the document in a popup, a
//! tree shown flat with a level of indentation per nesting, the symbol the
//! caret is in selected; typing narrows the rows (speed search), Enter
//! moves the caret to the symbol's name.

use code_intel::{DocumentSymbol, IntelError, Point};
use editor::Editor;
use gpui::{Context, Entity};

use crate::completion::list::kind_icon;
use crate::list_popup::{ListItem, ListOptions, ListRow};
use crate::navigation::ListTargets;
use crate::view::CodeIntelView;

/// The hint when the document has no symbols.
pub const NO_STRUCTURE: &str = "No structure to show";
/// The hint when the services have no structure.
pub const STRUCTURE_NOT_SUPPORTED: &str = "The language services do not show a file structure";

/// The rows of a document's symbols, depth first, the targets of the rows
/// (where the caret goes) and the row of the innermost symbol that holds
/// `caret`.
pub fn rows(
    symbols: &[DocumentSymbol],
    caret: Point,
) -> (Vec<ListItem>, Vec<Point>, Option<usize>) {
    fn walk(
        symbols: &[DocumentSymbol],
        level: u8,
        caret: Point,
        rows: &mut Vec<ListItem>,
        targets: &mut Vec<Point>,
        current: &mut Option<usize>,
    ) {
        for symbol in symbols {
            if symbol.range.start <= caret && caret <= symbol.range.end {
                *current = Some(rows.len());
            }
            rows.push(ListItem::Row(ListRow {
                icon: kind_icon(Some(symbol.kind), level > 0),
                text: symbol.name.clone().into(),
                detail: symbol.detail.clone().map(Into::into),
                indent: level,
                ..ListRow::default()
            }));
            targets.push(symbol.selection_range.start);
            walk(&symbol.children, level + 1, caret, rows, targets, current);
        }
    }
    let (mut items, mut targets, mut current) = (Vec::new(), Vec::new(), None);
    walk(symbols, 0, caret, &mut items, &mut targets, &mut current);
    (items, targets, current)
}

impl CodeIntelView {
    /// File Structure for the document of an editor.
    pub(crate) fn show_structure(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let Some(query) = self.caret_query(editor, cx) else {
            return;
        };
        let (Some(document), Some(services)) = (self.document_of(editor), self.services().cloned())
        else {
            return;
        };
        let doc = document.id.clone();
        let title = doc
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let request = services.document_symbols(&doc);
        let (editor, offset, caret) = (editor.clone(), query.offset, query.at);
        self.nav.request = Some(cx.spawn(async move |this, cx| {
            let answer = request.await;
            let _ = this.update(cx, |v, cx| {
                v.nav.request = None;
                match answer {
                    Ok(symbols) if !symbols.is_empty() => {
                        let (items, targets, current) = rows(&symbols, caret);
                        let options = ListOptions {
                            title: title.map(Into::into),
                            name: "structure",
                            selected: current,
                            ..ListOptions::default()
                        };
                        v.open_list(
                            &editor,
                            offset,
                            items,
                            options,
                            ListTargets::Symbols(targets),
                            cx,
                        );
                    }
                    Ok(_) => v.show_hint(&editor, offset, NO_STRUCTURE, cx),
                    Err(IntelError::Unsupported) => {
                        v.show_hint(&editor, offset, STRUCTURE_NOT_SUPPORTED, cx)
                    }
                    Err(_) => v.show_hint(&editor, offset, NO_STRUCTURE, cx),
                }
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use code_intel::SymbolKind;

    use super::*;

    fn symbol(name: &str, from: usize, to: usize, children: Vec<DocumentSymbol>) -> DocumentSymbol {
        let mut s = DocumentSymbol::new(
            name,
            SymbolKind::Function,
            Point::new(from, 0)..Point::new(to, 0),
        );
        s.children = children;
        s
    }

    #[test]
    fn the_tree_is_flat_with_levels_and_the_caret_symbol_is_the_innermost() {
        let tree = vec![
            symbol("a", 0, 5, vec![symbol("b", 1, 2, Vec::new())]),
            symbol("c", 6, 9, Vec::new()),
        ];
        let (items, targets, current) = rows(&tree, Point::new(1, 3));
        let names: Vec<_> = items
            .iter()
            .map(|i| match i {
                ListItem::Row(r) => (r.text.to_string(), r.indent),
                ListItem::Separator => unreachable!(),
            })
            .collect();
        assert_eq!(names, [("a".into(), 0), ("b".into(), 1), ("c".into(), 0)]);
        assert_eq!(targets.len(), 3);
        assert_eq!(current, Some(1));
        assert_eq!(rows(&tree, Point::new(7, 0)).2, Some(2));
        assert_eq!(rows(&tree, Point::new(20, 0)).2, None);
    }
}
