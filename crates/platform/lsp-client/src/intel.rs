//! [`CodeIntel`] over the language servers of a project.

use std::ops::Range;
use std::sync::Arc;

use code_intel::{
    AppliedEdit, CodeAction, CodeActionContext, CodeIntel, CompletionItem, CompletionTrigger,
    Completions, DocumentId, EventHub, FileChange, FormatOptions, Hover, Intel, IntelEvent,
    Location, Point, Rope, ServerId, ServerStatus, SignatureContext, SignatureHelp, TextEdit,
    ready, unsupported,
};

use crate::spec::ServerSpec;

/// The language services of a project over its servers: each document
/// goes to the servers of its language, each request to the servers that
/// serve its feature. Servers start when the first document of theirs
/// opens.
pub struct LspCodeIntel {
    specs: Vec<ServerSpec>,
    events: EventHub,
}

impl LspCodeIntel {
    pub fn new(specs: Vec<ServerSpec>) -> Arc<Self> {
        Arc::new(Self {
            specs,
            events: EventHub::new(),
        })
    }

    /// The servers it runs.
    pub fn specs(&self) -> &[ServerSpec] {
        &self.specs
    }

    /// The settings or the customizers' answers changed: the running
    /// servers are told (`workspace/didChangeConfiguration`).
    pub fn configuration_changed(&self) {}

    fn status_of(spec: &ServerSpec) -> ServerStatus {
        let mut status = ServerStatus::new(spec.id.clone(), spec.name.clone());
        status.roots = vec![spec.root.clone()];
        status.icon = spec.icon.clone();
        status.features = spec.features;
        status
    }
}

impl CodeIntel for LspCodeIntel {
    fn did_open(&self, _: &DocumentId, _: &str, _: u64, _: &Rope) {}

    fn did_change(&self, _: &DocumentId, _: u64, _: &[AppliedEdit], _: &Rope) {}

    fn did_reload(&self, _: &DocumentId, _: u64, _: &Rope) {}

    fn did_save(&self, _: &DocumentId, _: &Rope) {}

    fn did_close(&self, _: &DocumentId) {}

    fn files_changed(&self, _: &[FileChange]) {}

    fn completion(&self, _: &DocumentId, _: Point, _: CompletionTrigger) -> Intel<Completions> {
        unsupported()
    }

    fn resolve_completion(&self, _: &DocumentId, item: CompletionItem) -> Intel<CompletionItem> {
        ready(Ok(item))
    }

    fn hover(&self, _: &DocumentId, _: Point) -> Intel<Option<Hover>> {
        unsupported()
    }

    fn signature_help(
        &self,
        _: &DocumentId,
        _: Point,
        _: SignatureContext,
    ) -> Intel<Option<SignatureHelp>> {
        unsupported()
    }

    fn definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn type_definition(&self, _: &DocumentId, _: Point) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn references(&self, _: &DocumentId, _: Point, _: bool) -> Intel<Vec<Location>> {
        unsupported()
    }

    fn code_actions(
        &self,
        _: &DocumentId,
        _: Range<Point>,
        _: CodeActionContext,
    ) -> Intel<Vec<CodeAction>> {
        unsupported()
    }

    fn resolve_code_action(&self, _: &DocumentId, action: CodeAction) -> Intel<CodeAction> {
        ready(Ok(action))
    }

    fn format(
        &self,
        _: &DocumentId,
        _: Option<Range<Point>>,
        _: FormatOptions,
    ) -> Intel<Vec<TextEdit>> {
        unsupported()
    }

    fn status(&self) -> Vec<ServerStatus> {
        self.specs.iter().map(Self::status_of).collect()
    }

    fn events(&self) -> flume::Receiver<IntelEvent> {
        self.events.subscribe()
    }

    fn restart(&self, _: Option<&ServerId>) {}

    fn stop(&self, _: &ServerId) {}

    fn shutdown(&self) -> Intel<()> {
        ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use code_intel::{Feature, Features, IntelError, ServerState};
    use futures::executor::block_on;
    use process::CommandLine;

    use super::*;

    #[test]
    fn the_skeleton_lists_its_servers_and_answers_nothing() {
        let ruff = ServerSpec::new("ruff", CommandLine::new("ruff").arg("server"), "/p")
            .with_languages(["python"])
            .with_features(Features::of(&[Feature::Diagnostics, Feature::Formatting]));
        assert!(ruff.serves("python"));
        assert!(format!("{ruff:?}").contains("ruff"));
        let intel: Arc<dyn CodeIntel> = LspCodeIntel::new(vec![ruff]);
        let status = intel.status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].state, ServerState::Stopped);
        assert!(status[0].features.contains(Feature::Formatting));
        let doc = DocumentId::new("/p/a.py");
        intel.did_open(&doc, "python", 1, &Rope::from_str("x = 1\n"));
        assert_eq!(
            block_on(intel.completion(&doc, Point::new(0, 1), CompletionTrigger::Invoked)),
            Err(IntelError::Unsupported)
        );
        assert_eq!(block_on(intel.shutdown()), Ok(()));
    }
}
