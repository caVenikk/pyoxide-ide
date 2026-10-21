//! Editor tabs that show something other than a file's editor (a diff):
//! the view gives the tab its title, icon, tooltip and preview state, and
//! its state for `workbench.state`; the workbench draws it in the editor
//! area like an editor and handles closing it.

use gpui::{AnyView, App, Entity, EntityId, FocusHandle, Focusable, Render, SharedString};
use ui::icons::IconName;

/// A view shown in an editor tab.
pub trait TabView: Render + Focusable {
    fn tab_title(&self, cx: &App) -> SharedString;
    fn tab_icon(&self, cx: &App) -> IconName;
    fn tab_tooltip(&self, _: &App) -> Option<SharedString> {
        None
    }
    /// A preview tab: its title is grey, and the next view of its kind
    /// takes its place instead of a new tab (the reference's preview tab).
    fn is_preview(&self, _: &App) -> bool {
        false
    }
    /// What the tab's view shows, for `workbench.state`.
    fn tab_state(&self, _: &App) -> serde_json::Value {
        serde_json::Value::Null
    }
    /// The main toolbar shows the tab's title while it is active, as it
    /// shows a file's path (not the Welcome tab's).
    fn title_in_toolbar(&self, _: &App) -> bool {
        true
    }
}

/// A tab's view whatever its type.
pub(crate) trait AnyTabView {
    fn view(&self) -> AnyView;
    fn entity_id(&self) -> EntityId;
    fn title(&self, cx: &App) -> SharedString;
    fn icon(&self, cx: &App) -> IconName;
    fn tooltip(&self, cx: &App) -> Option<SharedString>;
    fn is_preview(&self, cx: &App) -> bool;
    fn focus_handle(&self, cx: &App) -> FocusHandle;
    fn state(&self, cx: &App) -> serde_json::Value;
    fn title_in_toolbar(&self, cx: &App) -> bool;
    /// The entity, for the owner that knows its type.
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: TabView + 'static> AnyTabView for Entity<T> {
    fn view(&self) -> AnyView {
        self.clone().into()
    }

    fn entity_id(&self) -> EntityId {
        Entity::entity_id(self)
    }

    fn title(&self, cx: &App) -> SharedString {
        self.read(cx).tab_title(cx)
    }

    fn icon(&self, cx: &App) -> IconName {
        self.read(cx).tab_icon(cx)
    }

    fn tooltip(&self, cx: &App) -> Option<SharedString> {
        self.read(cx).tab_tooltip(cx)
    }

    fn is_preview(&self, cx: &App) -> bool {
        self.read(cx).is_preview(cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.read(cx).focus_handle(cx)
    }

    fn state(&self, cx: &App) -> serde_json::Value {
        self.read(cx).tab_state(cx)
    }

    fn title_in_toolbar(&self, cx: &App) -> bool {
        self.read(cx).title_in_toolbar(cx)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The view of a tab as `Entity<T>` when it is one.
pub(crate) fn downcast<T: 'static>(view: &dyn AnyTabView) -> Option<&Entity<T>> {
    view.as_any().downcast_ref::<Entity<T>>()
}
