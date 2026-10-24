//! Handlers of Go to Declaration's object targets by scheme (ADR 0022): a
//! target that is no place in a file (`{scheme, id, title}`, such as a
//! table of a database) goes to the handler its scheme registered. The
//! database module registers `db`.

use std::collections::HashMap;
use std::rc::Rc;

use code_intel_ui::code_intel::ObjectTarget;
use gpui::{App, Global, Window};

/// Opens an object target (selects it in a tool window, opens its source).
pub type ObjectTargetHandler = Rc<dyn Fn(&ObjectTarget, &mut Window, &mut App)>;

#[derive(Default)]
struct Handlers(HashMap<String, ObjectTargetHandler>);

impl Global for Handlers {}

/// Registers the handler of a scheme, in place of an earlier one.
pub fn register(cx: &mut App, scheme: &str, handler: ObjectTargetHandler) {
    cx.default_global::<Handlers>()
        .0
        .insert(scheme.to_owned(), handler);
}

/// The handler of a scheme.
pub fn handler(cx: &App, scheme: &str) -> Option<ObjectTargetHandler> {
    cx.try_global::<Handlers>()?.0.get(scheme).cloned()
}

/// Passes a target to the handler of its scheme; false when the scheme has
/// none.
pub fn open(target: &ObjectTarget, window: &mut Window, cx: &mut App) -> bool {
    match handler(cx, &target.scheme) {
        Some(handler) => {
            handler(target, window, cx);
            true
        }
        None => false,
    }
}
