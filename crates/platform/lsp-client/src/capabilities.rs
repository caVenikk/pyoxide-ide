//! What the client advertises in `initialize` and what it reads from the
//! server's answer.
//!
//! The client capabilities follow the reference's `LspClientCapabilities`
//! (IntelliJ Platform, `platform/lsp`) for the features Pyoxide has; what
//! it does not have yet (semantic tokens, inlay hints, folding, code
//! lenses, document links and colours, symbols, hierarchies, show document,
//! resource operations in workspace edits, completion item defaults) is
//! not advertised, so servers do not send it.

use std::path::Path;

use serde_json::{Value, json};

use crate::encoding::PositionEncoding;
use crate::spec::ServerSpec;
use crate::uri;

pub(crate) fn client_capabilities() -> Value {
    json!({
        "general": {
            "positionEncodings": PositionEncoding::OFFERED,
            "staleRequestSupport": { "cancel": true, "retryOnContentModified": [] },
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": {
                "documentChanges": true,
                "failureHandling": "abort",
                "normalizesLineEndings": true,
            },
            "didChangeConfiguration": { "dynamicRegistration": false },
            "didChangeWatchedFiles": { "dynamicRegistration": true, "relativePatternSupport": true },
            "executeCommand": { "dynamicRegistration": false },
            "workspaceFolders": true,
            "configuration": true,
            "diagnostics": { "refreshSupport": true },
        },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "willSave": false,
                "willSaveWaitUntil": false,
                "didSave": true,
            },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "documentationFormat": ["markdown", "plaintext"],
                    "deprecatedSupport": true,
                    "tagSupport": { "valueSet": [1] },
                    "insertReplaceSupport": true,
                    "resolveSupport": { "properties": ["documentation"] },
                    "labelDetailsSupport": true,
                },
                "completionItemKind": { "valueSet": (1..=25).collect::<Vec<u32>>() },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "dynamicRegistration": true,
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                    "activeParameterSupport": true,
                },
                "contextSupport": true,
            },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "references": { "dynamicRegistration": true },
            "documentHighlight": { "dynamicRegistration": true },
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": ["quickfix", "", "source", "refactor", "refactor.inline", "refactor.extract"],
                    },
                },
                "isPreferredSupport": true,
                "disabledSupport": true,
                "dataSupport": true,
                "resolveSupport": { "properties": ["edit"] },
            },
            "formatting": { "dynamicRegistration": true },
            "rangeFormatting": { "dynamicRegistration": true },
            "rename": { "dynamicRegistration": true, "prepareSupport": true },
            "publishDiagnostics": {
                "relatedInformation": false,
                "versionSupport": true,
                "tagSupport": { "valueSet": [1, 2] },
                "codeDescriptionSupport": true,
                "dataSupport": true,
            },
            "diagnostic": { "dynamicRegistration": true, "relatedDocumentSupport": false },
        },
        "window": {
            "workDoneProgress": true,
            "showMessage": { "messageActionItem": { "additionalPropertiesSupport": false } },
            "showDocument": { "support": false },
        },
    })
}

/// The `initialize` parameters for a server (the reference's
/// `LspClientDescriptor.createInitializeParams`: client info, one root as
/// `rootUri`, `rootPath` and the only workspace folder, the initialization
/// options). The process id lets a server exit when the application dies.
pub(crate) fn initialize_params(spec: &ServerSpec) -> Value {
    let root = uri::from_path(&spec.root);
    let mut params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": app_info::full_name(), "version": env!("CARGO_PKG_VERSION") },
        "rootPath": spec.root.to_string_lossy(),
        "rootUri": root.as_str(),
        "capabilities": client_capabilities(),
        "trace": "off",
        "workspaceFolders": [workspace_folder(&spec.root)],
    });
    if let Some(options) = &spec.initialization_options {
        params["initializationOptions"] = options.clone();
    }
    params
}

/// A workspace folder: its URI and its name (the folder's name).
pub(crate) fn workspace_folder(root: &Path) -> Value {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    json!({ "uri": uri::from_path(root).as_str(), "name": name })
}

/// How a server wants document changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncKind {
    None,
    Full,
    Incremental,
}

/// What a server said about itself in its `initialize` answer, read
/// leniently (a server that sends a field in a shape this client does not
/// know loses that feature, nothing else).
#[derive(Debug, Clone)]
pub(crate) struct ServerInfo {
    pub name: Option<String>,
    pub version: Option<String>,
    pub encoding: PositionEncoding,
    pub capabilities: Value,
}

impl ServerInfo {
    pub fn from_initialize_result(result: &Value) -> Self {
        let capabilities = result
            .get("capabilities")
            .filter(|c| c.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let info = result.get("serverInfo");
        let text = |key: &str| {
            info.and_then(|i| i.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        Self {
            name: text("name"),
            version: text("version"),
            encoding: PositionEncoding::negotiated(
                capabilities.get("positionEncoding").and_then(Value::as_str),
            ),
            capabilities,
        }
    }

    /// A `...Provider` capability is present and not `false`.
    pub fn provides(&self, provider: &str) -> bool {
        match self.capabilities.get(provider) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(_) => true,
        }
    }

    /// An option of a provider object (`completionProvider.resolveProvider`).
    pub fn option(&self, provider: &str, option: &str) -> Option<&Value> {
        self.capabilities.get(provider)?.get(option)
    }

    /// The characters of a provider's `triggerCharacters` (or another list
    /// of strings).
    pub fn strings(&self, provider: &str, option: &str) -> Vec<String> {
        self.option(provider, option)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn sync_kind(&self) -> SyncKind {
        let kind = match self.capabilities.get("textDocumentSync") {
            Some(Value::Number(n)) => n.as_u64(),
            Some(Value::Object(sync)) => match sync.get("change") {
                Some(change) => change.as_u64(),
                None => Some(0),
            },
            _ => Some(0),
        };
        match kind {
            Some(1) => SyncKind::Full,
            Some(2) => SyncKind::Incremental,
            _ => SyncKind::None,
        }
    }

    /// The server wants `didOpen` and `didClose`.
    pub fn open_close(&self) -> bool {
        match self.capabilities.get("textDocumentSync") {
            Some(Value::Number(n)) => n.as_u64().is_some_and(|n| n > 0),
            Some(Value::Object(sync)) => {
                sync.get("openClose").and_then(Value::as_bool) == Some(true)
            }
            _ => false,
        }
    }

    /// `Some(include_text)` when the server wants `didSave`.
    pub fn save(&self) -> Option<bool> {
        let Some(Value::Object(sync)) = self.capabilities.get("textDocumentSync") else {
            return None;
        };
        match sync.get("save")? {
            Value::Bool(true) => Some(false),
            Value::Object(options) => Some(
                options
                    .get("includeText")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use process::CommandLine;

    use super::*;

    #[test]
    fn the_capabilities_are_valid_lsp() {
        let caps: lsp_types::ClientCapabilities =
            serde_json::from_value(client_capabilities()).unwrap();
        let general = caps.general.unwrap();
        assert_eq!(
            general
                .position_encodings
                .unwrap()
                .iter()
                .map(|e| e.as_str().to_owned())
                .collect::<Vec<_>>(),
            ["utf-8", "utf-16"]
        );
        let workspace = caps.workspace.unwrap();
        assert_eq!(workspace.configuration, Some(true));
        assert_eq!(workspace.apply_edit, Some(true));
        assert_eq!(workspace.workspace_folders, Some(true));
        assert_eq!(
            workspace
                .did_change_watched_files
                .unwrap()
                .relative_pattern_support,
            Some(true)
        );
        let text = caps.text_document.unwrap();
        let item = text.completion.unwrap().completion_item.unwrap();
        assert_eq!(item.snippet_support, Some(true));
        assert_eq!(item.label_details_support, Some(true));
        assert!(text.diagnostic.is_some());
        assert_eq!(text.rename.unwrap().prepare_support, Some(true));
        assert_eq!(caps.window.unwrap().work_done_progress, Some(true));
    }

    #[test]
    fn initialize_parameters() {
        let root = if cfg!(windows) {
            r"C:\work\demo"
        } else {
            "/work/demo"
        };
        let mut spec = ServerSpec::new("ruff", CommandLine::new("ruff").arg("server"), root);
        spec.initialization_options = Some(json!({"settings": {"showSyntaxErrors": false}}));
        let params = initialize_params(&spec);
        let parsed: lsp_types::InitializeParams = serde_json::from_value(params.clone()).unwrap();
        assert_eq!(parsed.process_id, Some(std::process::id()));
        assert_eq!(parsed.client_info.unwrap().name, "App");
        let folders = parsed.workspace_folders.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "demo");
        assert_eq!(params["rootUri"], folders[0].uri.as_str());
        assert_eq!(params["rootPath"], root);
        assert_eq!(
            params["initializationOptions"]["settings"]["showSyntaxErrors"],
            false
        );
    }

    #[test]
    fn server_answers_read_leniently() {
        let ruff = ServerInfo::from_initialize_result(&json!({
            "capabilities": {
                "positionEncoding": "utf-8",
                "textDocumentSync": {"openClose": true, "change": 2},
                "codeActionProvider": {"codeActionKinds": ["quickfix"], "resolveProvider": true},
                "diagnosticProvider": {"identifier": "Ruff", "interFileDependencies": false, "workspaceDiagnostics": false},
                "hoverProvider": true,
                "renameProvider": false,
                "completionProvider": {"triggerCharacters": [".", "\""]},
            },
            "serverInfo": {"name": "ruff", "version": "0.15.20"},
        }));
        assert_eq!(ruff.name.as_deref(), Some("ruff"));
        assert_eq!(ruff.version.as_deref(), Some("0.15.20"));
        assert_eq!(ruff.encoding, PositionEncoding::Utf8);
        assert_eq!(ruff.sync_kind(), SyncKind::Incremental);
        assert!(ruff.open_close());
        assert_eq!(ruff.save(), None);
        assert!(ruff.provides("diagnosticProvider"));
        assert!(ruff.provides("hoverProvider"));
        assert!(!ruff.provides("renameProvider"));
        assert!(!ruff.provides("definitionProvider"));
        assert_eq!(
            ruff.option("codeActionProvider", "resolveProvider"),
            Some(&json!(true))
        );
        assert_eq!(
            ruff.strings("completionProvider", "triggerCharacters"),
            [".", "\""]
        );

        let old = ServerInfo::from_initialize_result(&json!({
            "capabilities": {"textDocumentSync": 1, "positionEncoding": 42},
        }));
        assert_eq!(old.encoding, PositionEncoding::Utf16);
        assert_eq!(old.sync_kind(), SyncKind::Full);
        assert!(old.open_close());
        assert_eq!(old.name, None);

        let saving = ServerInfo::from_initialize_result(&json!({
            "capabilities": {"textDocumentSync": {"save": {"includeText": true}}},
        }));
        assert_eq!(saving.save(), Some(true));
        assert_eq!(saving.sync_kind(), SyncKind::None);
        assert!(!saving.open_close());

        let broken = ServerInfo::from_initialize_result(&json!("nonsense"));
        assert_eq!(broken.encoding, PositionEncoding::Utf16);
        assert_eq!(broken.sync_kind(), SyncKind::None);
    }
}
