//! The Documentation tool window of the main window (#338): the view of
//! the window's code intelligence (`code_intel_ui::doc_window`), its
//! stripe button added in the right area by its first use
//! (`DocumentationToolWindowManager`: anchor right, `shouldBeAvailable`
//! false), shown without taking the focus (`toolWindow.show()`).

use code_intel_ui::doc_window::{self, DocToolWindow};
use gpui::{Context, Window};
use ui::icons::IconName;

use super::Workbench;
use crate::tool_windows::{Area, ToolWindow, ToolWindowContent, ToolWindowSpec, ToolWindowView};

/// The Documentation tool window's id.
pub const DOCUMENTATION: ToolWindow = ToolWindow(doc_window::ID);

impl ToolWindowView for DocToolWindow {}

impl Workbench {
    /// Shows the Documentation tool window in the right area, registering
    /// it at its first use; the focus stays where it is.
    pub(super) fn show_documentation_window(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(code_intel) = self.code_intel.clone() else {
            return;
        };
        if self.tool_windows.get(DOCUMENTATION).is_none() {
            let view = code_intel.read(cx).doc_window().clone();
            self.tool_windows.register(
                ToolWindowSpec {
                    id: DOCUMENTATION,
                    area: Area::Right,
                    order: 10,
                    icon: IconName::TOOL_DOCUMENTATION,
                    title: doc_window::TITLE,
                    command: None,
                    registered: true,
                },
                Some(ToolWindowContent::new(view)),
            );
        }
        self.tool_windows.show(DOCUMENTATION);
        cx.notify();
    }
}
