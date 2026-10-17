//! The debugger of the window (ADR 0015): the product's debug hook drives
//! a [`DebuggerView`] that opens the project's debugger, keeps the
//! breakpoints of the editor tabs in their gutters, and shows the
//! breakpoint properties popup and the Breakpoints dialog; Jump to Source
//! of the dialog opens the file here.

use std::path::Path;
use std::rc::Rc;

use debugger_ui::{DebuggerView, DebuggerViewEvent};
use editor::Editor;
use gpui::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement, Subscription, Window,
};
use serde_json::Value as Json;

use super::Workbench;
use crate::app::Product;
use crate::file_types::FileTypes;

impl Workbench {
    /// The window's debugger UI and its subscriptions.
    pub(super) fn new_debugger_view(
        product: &Product,
        file_types: Rc<FileTypes>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<DebuggerView>, Vec<Subscription>) {
        let view = cx.new(|cx| {
            let mut view = DebuggerView::new(product.debug.clone(), cx);
            view.set_activate(!crate::env::background());
            view.set_highlighters(Rc::new(move |path: &Path| {
                file_types
                    .for_path(path)
                    .and_then(|t| t.highlighter.as_ref())
                    .map(|factory| factory())
            }));
            view
        });
        let subscriptions = vec![
            cx.observe(&view, |_, _, cx| cx.notify()),
            cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
                DebuggerViewEvent::OpenAt { path, line } => {
                    window.activate_window();
                    let point = text::Point {
                        line: line.saturating_sub(1) as usize,
                        column: 0,
                    };
                    this.open_file_at(path.clone(), point, None, window, cx);
                }
            }),
        ];
        (view, subscriptions)
    }

    /// The window's debugger UI.
    pub fn debugger_view(&self) -> &Entity<DebuggerView> {
        &self.debugger
    }

    pub(super) fn debug_project_opened(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debugger
            .update(cx, |v, cx| v.project_opened(root, window, cx));
    }

    pub(super) fn debug_tab_opened(
        &mut self,
        editor: &Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.debugger
            .update(cx, |v, cx| v.editor_opened(editor, window, cx));
    }

    pub(super) fn debug_tab_closed(&mut self, editor: &Entity<Editor>, cx: &mut Context<Self>) {
        self.debugger
            .update(cx, |v, cx| v.editor_closed(editor, cx));
    }

    /// A file or folder was renamed or moved: its breakpoints follow.
    pub(super) fn debug_files_moved(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        let moves = [(from.to_owned(), to.to_owned())];
        self.debugger.update(cx, |v, cx| v.files_moved(&moves, cx));
    }

    /// The breakpoint commands, run with the active editor wherever the
    /// focus is in the window.
    pub(super) fn debug_actions<E: InteractiveElement>(&self, root: E) -> E {
        debugger_ui::forward_actions(root, self.debugger.clone(), self.active_editor().cloned())
    }

    /// The breakpoint popup and the gutter tooltip over the active editor.
    pub(super) fn debug_overlay(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let active = self.active_editor().cloned();
        self.debugger
            .update(cx, |v, cx| v.render_overlay(active.as_ref(), window, cx))
    }

    /// `debug.state`.
    pub fn debug_state(&self, cx: &gpui::App) -> Json {
        self.debugger.read(cx).state_json(cx)
    }
}
