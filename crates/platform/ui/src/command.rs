//! UI commands (ADR 0007): user actions that need the UI thread and the
//! window, declared once with [`ui_commands!`](crate::ui_commands).
//!
//! A declaration creates a GPUI action whose action name is the command id
//! (`editor.undo`), so a keymap entry, a menu item, a test and the control
//! channel all reach the same handler, and it lists the command's metadata
//! for the command index. Components handle their actions with GPUI's
//! `on_action`; [`invoke`] runs a command by id with JSON arguments.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{App, Global, Window};
use serde_json::Value as Json;

pub use commands::{CommandError, CommandMeta};

/// A declared UI command.
#[derive(Debug, Clone)]
pub struct UiCommand {
    pub meta: CommandMeta,
    /// Key context of the component that handles it (`Editor`, `Workbench`,
    /// ...); documentation for keymaps and agents.
    pub scope: &'static str,
}

/// A command that answers from the current state instead of changing it.
pub type QueryFn = Rc<dyn Fn(Json, &mut Window, &mut App) -> Result<Json, CommandError>>;

/// Every registered UI command, by id.
#[derive(Default)]
pub struct CommandIndex {
    commands: BTreeMap<&'static str, UiCommand>,
    queries: BTreeMap<&'static str, QueryFn>,
}

impl Global for CommandIndex {}

impl CommandIndex {
    pub fn get(&self, id: &str) -> Option<&UiCommand> {
        self.commands.get(id)
    }

    /// All commands sorted by id.
    pub fn list(&self) -> impl Iterator<Item = &UiCommand> {
        self.commands.values()
    }
}

/// Adds declared commands to the index. Panics on a duplicate id with a
/// different declaration (a programming error).
pub fn register(cx: &mut App, commands: &[UiCommand]) {
    let index = cx.default_global::<CommandIndex>();
    for command in commands {
        if let Some(old) = index.commands.insert(command.meta.id, command.clone()) {
            assert!(
                old.meta == command.meta,
                "duplicate command id {}",
                command.meta.id
            );
        }
    }
}

/// Registers the handler of a query command (declared with
/// [`ui_commands!`](crate::ui_commands) like any other).
pub fn register_query(cx: &mut App, id: &'static str, query: QueryFn) {
    cx.default_global::<CommandIndex>()
        .queries
        .insert(id, query);
}

/// The command answers from the state instead of changing it (a query).
pub fn is_query(cx: &App, id: &str) -> bool {
    cx.try_global::<CommandIndex>()
        .is_some_and(|index| index.queries.contains_key(id))
}

/// All registered commands, sorted by id.
pub fn list(cx: &App) -> Vec<UiCommand> {
    cx.try_global::<CommandIndex>()
        .map(|index| index.list().cloned().collect())
        .unwrap_or_default()
}

/// Runs the command `id` in `window`.
///
/// Query commands return their answer. Other commands are dispatched as
/// their GPUI action to the focused element, exactly like a key binding, and
/// return `null`; their effect is visible after the dispatch has run (the
/// next turn of the event loop). Errors: `not_found` (unknown id),
/// `invalid_args` (arguments do not match), `not_available` (no element on
/// the focus path handles the command).
pub fn invoke(
    id: &str,
    args: Json,
    window: &mut Window,
    cx: &mut App,
) -> Result<Json, CommandError> {
    let index = cx
        .try_global::<CommandIndex>()
        .ok_or_else(|| CommandError::not_found(format!("unknown command {id}")))?;
    if index.get(id).is_none() {
        return Err(CommandError::not_found(format!("unknown command {id}")));
    }
    if let Some(query) = index.queries.get(id).cloned() {
        return query(args, window, cx);
    }
    let args = if args.is_null() {
        Json::Object(Default::default())
    } else {
        args
    };
    let action = cx
        .build_action(id, Some(args))
        .map_err(|e| CommandError::invalid_args(format!("{id}: {e}")))?;
    // A window with a dialog open takes no input: the command goes to the
    // dialog, or to a popup open over it (queries above still read the
    // window's own state).
    let target = crate::dialog_window::innermost(window.window_handle(), cx);
    if target != window.window_handle() {
        return target
            .update(cx, |_, window, cx| {
                if !window.is_action_available(action.as_ref(), cx)
                    && !cx.is_action_available(action.as_ref())
                {
                    return Err(CommandError::new(
                        "not_available",
                        format!("{id} is not available in the open dialog"),
                    ));
                }
                window.dispatch_action(action, cx);
                Ok(Json::Null)
            })
            .map_err(|e| CommandError::new("not_available", format!("{id}: {e}")))?;
    }
    // Window handlers on the focus path, then application-wide handlers
    // (`App::on_action`, such as quit).
    if !window.is_action_available(action.as_ref(), cx) && !cx.is_action_available(action.as_ref())
    {
        // A command of a non-modal dialog open from the window (the
        // Settings window): it runs there, or in a dialog open over it.
        for dialog in crate::dialog_window::non_modal_of(window.window_handle(), cx) {
            let target = crate::dialog_window::innermost(dialog, cx);
            let ran = target.update(cx, |_, window, cx| {
                let available = window.is_action_available(action.as_ref(), cx);
                if available {
                    window.dispatch_action(action.boxed_clone(), cx);
                }
                available
            });
            if ran.unwrap_or(false) {
                return Ok(Json::Null);
            }
        }
        return Err(CommandError::new(
            "not_available",
            format!("{id} is not available here"),
        ));
    }
    window.dispatch_action(action, cx);
    Ok(Json::Null)
}

/// Declares UI commands: one GPUI action per command, named by the command
/// id, and a constant list of their metadata.
///
/// ```ignore
/// ui::ui_commands! {
///     COMMANDS, scope = "Editor";
///     Undo => "editor.undo", "Undo", "Undoes the last group of edits.";
///     Type { text: String } => "editor.type", "Type Text",
///         "Inserts text at the carets.", r#"{"text": "abc"}"#;
/// }
/// ```
#[macro_export]
macro_rules! ui_commands {
    (
        $list:ident, scope = $scope:literal;
        $(
            $(#[doc = $doc:literal])*
            $name:ident $({ $($field:ident : $ty:ty),* $(,)? })?
                => $id:tt, $title:literal, $summary:literal $(, $args:literal)?;
        )*
    ) => {
        $(
            $crate::__ui_command_struct!(
                [$(#[doc = $doc])*] $name, $id $(, { $($field : $ty),* })?
            );
        )*

        /// Metadata of the commands declared here.
        pub const $list: &[$crate::command::UiCommand] = &[
            $(
                $crate::command::UiCommand {
                    meta: $crate::command::CommandMeta {
                        id: $id,
                        title: $title,
                        summary: $summary,
                        args_example: $crate::__ui_command_args!($($args)?),
                    },
                    scope: $scope,
                },
            )*
        ];
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ui_command_struct {
    ([$($doc:tt)*] $name:ident, $id:tt) => {
        $($doc)*
        #[derive(::std::clone::Clone, ::std::cmp::PartialEq, ::std::default::Default, ::std::fmt::Debug, gpui::Action)]
        #[action(name = $id)]
        pub struct $name;
    };
    ([$($doc:tt)*] $name:ident, $id:tt, { $($field:ident : $ty:ty),* }) => {
        $($doc)*
        #[derive(
            ::std::clone::Clone,
            ::std::cmp::PartialEq,
            ::std::fmt::Debug,
            gpui::private::serde::Deserialize,
            gpui::private::schemars::JsonSchema,
            gpui::Action,
        )]
        #[action(name = $id)]
        #[serde(crate = "gpui::private::serde", deny_unknown_fields)]
        #[schemars(crate = "gpui::private::schemars")]
        pub struct $name {
            $(pub $field: $ty,)*
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __ui_command_args {
    () => {
        "{}"
    };
    ($args:literal) => {
        $args
    };
}
