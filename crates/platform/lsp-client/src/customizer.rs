//! What a product adjusts in what its servers say.

use std::ops::Range;
use std::path::Path;

use code_intel::{
    CodeAction, CodeActionContext, CompletionItem, Diagnostic, DocumentId, Point, Rope,
    ServerStatus,
};
use serde_json::Value;

/// A product's adjustments to one server, as the reference's LSP
/// customizations (`LspDiagnosticsCustomizer`, `customizeQuickFixes`).
/// Every method has a default that changes nothing. Called on the client's
/// threads: no UI here.
pub trait ServerCustomizer: Send + Sync + 'static {
    /// The answer to one item of `workspace/configuration`: the `section`
    /// asked for (`python`, `basedpyright.analysis`; the whole settings
    /// when `None`) for a scope (a file or folder) from the spec's
    /// `settings`. By default the dotted section in `settings`
    /// ([`lookup_section`]).
    fn configuration(
        &self,
        section: Option<&str>,
        scope: Option<&Path>,
        settings: &Value,
    ) -> Value {
        let _ = scope;
        lookup_section(settings, section)
    }

    /// Adjusts a diagnostic of the server before it is reported: its
    /// severity, message or code (ruff's rule names instead of codes).
    fn diagnostic(&self, diagnostic: &mut Diagnostic) {
        let _ = diagnostic;
    }

    /// Adjusts the server's completion items for a document before they
    /// are shown, and an item again once it was resolved: how a language
    /// presents them (the tail and type text of a Python function), or
    /// which are offered.
    fn completion_items(&self, doc: &DocumentId, items: &mut Vec<CompletionItem>) {
        let _ = (doc, items);
    }

    /// Adjusts the server's code actions at a range: renames them, nests
    /// some under others (`children`), drops some, and adds the product's
    /// own actions (disable a rule for the file or the project). `text` is
    /// the document's text the request was made for (the editor's, not the
    /// file's on disk); the range and the actions' edits are in it.
    fn code_actions(
        &self,
        doc: &DocumentId,
        text: &Rope,
        range: Range<Point>,
        context: &CodeActionContext,
        actions: &mut Vec<CodeAction>,
    ) {
        let _ = (doc, text, range, context, actions);
    }

    /// The server answered `initialize`: its name and version are known.
    fn initialized(&self, status: &ServerStatus) {
        let _ = status;
    }
}

/// A server without adjustments.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCustomization;

impl ServerCustomizer for NoCustomization {}

/// The value at a dotted section of settings (`basedpyright.analysis`);
/// the whole settings for `None` and `null` when a part is missing.
pub fn lookup_section(settings: &Value, section: Option<&str>) -> Value {
    let Some(section) = section.filter(|s| !s.is_empty()) else {
        return settings.clone();
    };
    section
        .split('.')
        .try_fold(settings, |value, key| value.get(key))
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sections_of_settings() {
        let settings = json!({
            "basedpyright": { "analysis": { "typeCheckingMode": "standard" } },
            "python": { "pythonPath": "/p/.venv/bin/python" },
        });
        assert_eq!(
            lookup_section(&settings, Some("basedpyright.analysis")),
            json!({ "typeCheckingMode": "standard" })
        );
        assert_eq!(
            lookup_section(&settings, Some("python.pythonPath")),
            json!("/p/.venv/bin/python")
        );
        assert_eq!(lookup_section(&settings, Some("ty")), Value::Null);
        assert_eq!(lookup_section(&settings, None), settings);
        assert_eq!(
            NoCustomization.configuration(Some("python"), None, &settings),
            json!({ "pythonPath": "/p/.venv/bin/python" })
        );
    }
}
