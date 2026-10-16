//! Settings pages (`Configurable`): the registry of pages by stable dotted
//! id and the trait a page's view implements. A page edits a draft:
//! Apply hands it to its owner, "Revert changes" goes back to the stored
//! state, Cancel drops it.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, FocusHandle, Global, Render, Subscription,
    Window,
};
use serde_json::Value as Json;

/// Where a page's options are stored (ADR 0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `settings.toml` of the application.
    Application,
    /// `.oxide/project.toml`, shared with the project.
    ProjectShared,
    /// `.oxide/workspace.toml`, the person's own for the project.
    ProjectPersonal,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Scope::Application => "application",
            Scope::ProjectShared => "project-shared",
            Scope::ProjectPersonal => "project-personal",
        }
    }

    /// The tree marks pages with project options (`ConfigurableTreeRenderer`
    /// project icon).
    pub fn is_project(self) -> bool {
        self != Scope::Application
    }
}

/// What a page is opened for.
#[derive(Debug, Clone, Default)]
pub struct PageContext {
    /// The project folder of the window the Settings window was opened
    /// from.
    pub project: Option<PathBuf>,
}

/// Creates a page's view for a Settings window.
pub type CreatePage = Rc<dyn Fn(&PageContext, &mut Window, &mut App) -> PageHandle>;

/// A page of the tree.
#[derive(Clone)]
pub struct PageSpec {
    /// Stable dotted id (`appearance`, `system`, `editor.font`).
    pub id: &'static str,
    /// The group it is listed under; `None` at the top level.
    pub parent: Option<&'static str>,
    pub title: &'static str,
    /// Order among its siblings, the larger first (the reference's
    /// `groupWeight`); equal weights keep the order of registration.
    pub weight: i32,
    pub scope: Scope,
    /// Words the search finds the page by besides its title: the labels of
    /// its options.
    pub search_terms: &'static [&'static str],
    /// A group page without a view of its own shows this and links to its
    /// children.
    pub description: Option<&'static str>,
    /// The page's view; `None` for a group page.
    pub create: Option<CreatePage>,
}

impl std::fmt::Debug for PageSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageSpec")
            .field("id", &self.id)
            .field("parent", &self.parent)
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl PageSpec {
    /// A group page with a description and links to its children.
    pub fn group(id: &'static str, parent: Option<&'static str>, title: &'static str) -> Self {
        Self {
            id,
            parent,
            title,
            weight: 0,
            scope: Scope::Application,
            search_terms: &[],
            description: None,
            create: None,
        }
    }
}

#[derive(Default)]
struct Pages(Vec<PageSpec>);

impl Global for Pages {}

/// Adds pages to the tree; a page with the id of a registered one replaces
/// it.
pub fn register(cx: &mut App, pages: impl IntoIterator<Item = PageSpec>) {
    let list = &mut cx.default_global::<Pages>().0;
    for page in pages {
        match list.iter_mut().find(|p| p.id == page.id) {
            Some(old) => *old = page,
            None => list.push(page),
        }
    }
}

/// Every registered page, in registration order.
pub fn pages(cx: &App) -> Vec<PageSpec> {
    cx.try_global::<Pages>()
        .map(|p| p.0.clone())
        .unwrap_or_default()
}

/// The view of a settings page.
pub trait SettingsPage: Render + Sized {
    /// The draft differs from the stored state.
    fn is_modified(&self, cx: &App) -> bool;
    /// Stores the draft; an error message keeps the window open on the
    /// page.
    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String>;
    /// Goes back to the stored state ("Revert changes", and when the window
    /// is shown again).
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>);
    /// What takes the focus when the page is entered.
    fn focus(&self, _cx: &App) -> Option<FocusHandle> {
        None
    }
    /// The page's fields for `settings.state`.
    fn state(&self, _cx: &App) -> Json {
        Json::Null
    }
    /// Changes a field of the draft (`settings.edit`).
    fn edit(
        &mut self,
        field: &str,
        _value: &Json,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Result<(), String> {
        Err(format!("no field `{field}` on this page"))
    }
    /// A page with a search of its own (Keymap) takes the window's search
    /// text into it when the page is shown from a search that did not find
    /// it by name (`SearchableConfigurable.enableSearch`); `""` when the
    /// search is cleared. Pages without one light up their labels instead
    /// ([`crate::search::label`]).
    fn search(&mut self, _text: &str, _window: &mut Window, _cx: &mut Context<Self>) {}
    /// Whether the page has a search of its own ([`Self::search`]).
    fn has_search(&self) -> bool {
        false
    }
}

/// Called when a page's view changes.
type OnChange = Box<dyn Fn(&mut Window, &mut App)>;

/// A page's view seen without its type.
trait ErasedPage {
    fn is_modified(&self, cx: &App) -> bool;
    fn apply(&self, window: &mut Window, cx: &mut App) -> Result<(), String>;
    fn reset(&self, window: &mut Window, cx: &mut App);
    fn focus(&self, cx: &App) -> Option<FocusHandle>;
    fn state(&self, cx: &App) -> Json;
    fn edit(
        &self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String>;
    fn observe(&self, window: &mut Window, cx: &mut App, on_change: OnChange) -> Subscription;
    fn search(&self, text: &str, window: &mut Window, cx: &mut App);
    fn has_search(&self, cx: &App) -> bool;
}

impl<V: SettingsPage + 'static> ErasedPage for Entity<V> {
    fn is_modified(&self, cx: &App) -> bool {
        self.read(cx).is_modified(cx)
    }

    fn apply(&self, window: &mut Window, cx: &mut App) -> Result<(), String> {
        self.update(cx, |p, cx| p.apply(window, cx))
    }

    fn reset(&self, window: &mut Window, cx: &mut App) {
        self.update(cx, |p, cx| p.reset(window, cx))
    }

    fn focus(&self, cx: &App) -> Option<FocusHandle> {
        self.read(cx).focus(cx)
    }

    fn state(&self, cx: &App) -> Json {
        self.read(cx).state(cx)
    }

    fn edit(
        &self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String> {
        self.update(cx, |p, cx| p.edit(field, value, window, cx))
    }

    fn observe(&self, window: &mut Window, cx: &mut App, on_change: OnChange) -> Subscription {
        window.observe(self, cx, move |_, window, cx| on_change(window, cx))
    }

    fn search(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.update(cx, |p, cx| p.search(text, window, cx))
    }

    fn has_search(&self, cx: &App) -> bool {
        self.read(cx).has_search()
    }
}

/// A page's view with its draft operations, independent of its type.
#[derive(Clone)]
pub struct PageHandle {
    pub(crate) view: AnyView,
    page: Rc<dyn ErasedPage>,
}

impl PageHandle {
    pub fn new<V: SettingsPage + 'static>(page: Entity<V>) -> Self {
        Self {
            view: page.clone().into(),
            page: Rc::new(page),
        }
    }

    pub fn is_modified(&self, cx: &App) -> bool {
        self.page.is_modified(cx)
    }

    pub fn apply(&self, window: &mut Window, cx: &mut App) -> Result<(), String> {
        self.page.apply(window, cx)
    }

    pub fn reset(&self, window: &mut Window, cx: &mut App) {
        self.page.reset(window, cx)
    }

    pub fn focus(&self, cx: &App) -> Option<FocusHandle> {
        self.page.focus(cx)
    }

    pub fn state(&self, cx: &App) -> Json {
        self.page.state(cx)
    }

    pub fn edit(
        &self,
        field: &str,
        value: &Json,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(), String> {
        self.page.edit(field, value, window, cx)
    }

    /// Hands the window's search text to the page's own search.
    pub fn search(&self, text: &str, window: &mut Window, cx: &mut App) {
        self.page.search(text, window, cx)
    }

    /// The page has a search of its own.
    pub fn has_search(&self, cx: &App) -> bool {
        self.page.has_search(cx)
    }

    /// Calls `on_change` whenever the page's view changes.
    pub(crate) fn observe(
        &self,
        window: &mut Window,
        cx: &mut App,
        on_change: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Subscription {
        self.page.observe(window, cx, Box::new(on_change))
    }
}

/// A page created from a view constructor (the usual [`PageSpec::create`]).
pub fn create<V: SettingsPage + 'static>(
    build: fn(&PageContext, &mut Window, &mut Context<V>) -> V,
) -> CreatePage {
    Rc::new(move |context, window, cx| {
        let context = context.clone();
        PageHandle::new(cx.new(|cx| build(&context, window, cx)))
    })
}
