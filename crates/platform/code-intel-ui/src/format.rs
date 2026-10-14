//! Reformat Code and Optimize Imports through the service that has the
//! feature, as the reference's `LspFormattingService` and its import
//! optimizer do: the whole file, or the selection with range formatting;
//! the imports by the `source.organizeImports` code action. Each change is
//! one undo step. Without a formatter the status bar says so.

use code_intel::{CodeActionContext, CodeActionKind, FormatOptions, IntelError};
use editor::Editor;
use gpui::{Context, Entity};

use crate::view::{CodeIntelEvent, CodeIntelView};

/// The notice when no service formats the file.
pub const NO_FORMATTER: &str = "No formatter is available for this file";

impl CodeIntelView {
    /// Reformat Code in an editor: the selected lines, or the whole file.
    pub(crate) fn reformat_editor(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let (Some(query), Some(services)) =
            (self.caret_query(editor, cx), self.services().cloned())
        else {
            return;
        };
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let selection = editor.read(cx).selections().primary().range();
        let range = (!selection.is_empty()).then(|| {
            document.versions.point(selection.start)..document.versions.point(selection.end)
        });
        let options = FormatOptions::default();
        let request = services.format(&query.doc, range, options);
        let (version, path) = (query.version, query.path);
        self.format_request = Some(cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |v, cx| {
                v.format_request = None;
                match result {
                    Ok(edits) => {
                        let edit = code_intel::WorkspaceEdit {
                            files: vec![code_intel::FileEdits {
                                path: path.clone(),
                                edits,
                            }],
                        };
                        v.apply_workspace_edit(&edit, Some((&path, version)), cx);
                    }
                    Err(IntelError::Unsupported) => {
                        cx.emit(CodeIntelEvent::Notice(NO_FORMATTER.to_owned()))
                    }
                    Err(_) => {}
                }
            });
        }));
    }

    /// Optimize Imports in an editor: the organize imports action of the
    /// file's services.
    pub(crate) fn optimize_imports_in(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        let (Some(query), Some(services)) =
            (self.caret_query(editor, cx), self.services().cloned())
        else {
            return;
        };
        let Some(document) = self.document_of(editor) else {
            return;
        };
        let len = document.versions.text().len_chars();
        let whole = document.versions.point(0)..document.versions.point(len);
        let request =
            services.code_actions(&query.doc, whole, CodeActionContext::organize_imports());
        let (editor, version) = (editor.clone(), query.version);
        self.format_request = Some(cx.spawn(async move |this, cx| {
            let actions = request.await.unwrap_or_default();
            let organize = CodeActionKind::organize_imports();
            let Some(action) = actions.into_iter().find(|a| a.kind.matches(&organize)) else {
                return;
            };
            let _ = this.update(cx, |v, cx| {
                v.format_request = None;
                v.apply_code_action(&editor, version, action, cx);
            });
        }));
    }
}
