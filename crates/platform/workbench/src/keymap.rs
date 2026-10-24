//! Keymaps (ADR 0007): TOML files that bind keystrokes to command ids, per
//! key context. Two default keymaps ship, one for macOS and one for Windows
//! and Linux; their keystrokes follow the reference IDE's default keymaps
//! (the IntelliJ Platform Community keymap files, Apache-2.0, see NOTICE).
//!
//! ```toml
//! [[context]]
//! name = "Editor"          # a GPUI key context predicate; omit for global
//! [context.bindings]
//! "cmd-z" = "editor.undo"
//! "cmd-alt-1" = { command = "tabs.select", args = { index = 0 } }
//! ```
//!
//! Keystrokes use GPUI's syntax (`cmd-shift-z`, `alt-left`, `ctrl-f4`).
//! A modifier written twice (`"shift shift"`) is a double tap of that key
//! ([`gesture`]), bound everywhere; `button4` and `button5` are the mouse's
//! back and forward buttons (the reference keymaps' names), bound
//! everywhere in the window.
//!
//! A default keymap is a folder of files, one per area (`keymaps/macos/`,
//! `keymaps/default/`: `editor.toml`, `search.toml`, ...), so that two
//! features seldom edit the same file; [`MACOS`] and [`DEFAULT`] join them in
//! a fixed order. The order is part of the keymap: a deeper context wins,
//! and of two bindings of one keystroke whose innermost context is the
//! same (a binding without a context counts as the innermost one) the later
//! one wins, so a popup's Escape comes after the editor's. An area's file is
//! placed after the files whose bindings it must win over; a new area file
//! is added to both lists.
//!
//! A module (ADR 0022) brings its own folders of area files in its crate,
//! joined the same way into a [`KeymapFiles`]; they come after the
//! workbench's files ([`active::set_modules`]).
//!
//! Besides the two base keymaps there are derived ones (ADR 0018,
//! [`derived`]): the reference's VS Code keymaps and custom keymaps
//! imported from the reference ([`reference`]), converted through the
//! action table ([`actions`]). [`active`] keeps the keymap in use and
//! switches it.

pub mod actions;
pub mod active;
pub mod derived;
mod gesture;
pub mod reference;

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{
    Action, App, DummyKeyboardMapper, Global, KeyBinding, KeyBindingContextPredicate, Keystroke,
    Modifiers, MouseButton, NavigationDirection, Window,
};
use serde::Deserialize;
use serde_json::Value as Json;

pub use gesture::{DoubleTap, ModifierKey, double_tap_text, parse_double_tap};

/// The default keymap of macOS: the files of `keymaps/macos/`, one per area,
/// joined in this order (see the module documentation).
pub const MACOS: &str = concat!(
    include_str!("../keymaps/macos/editor.toml"),
    include_str!("../keymaps/macos/code-insight.toml"),
    include_str!("../keymaps/macos/window.toml"),
    include_str!("../keymaps/macos/run.toml"),
    include_str!("../keymaps/macos/debug.toml"),
    include_str!("../keymaps/macos/vcs-markers.toml"),
    include_str!("../keymaps/macos/vcs-branches.toml"),
    include_str!("../keymaps/macos/vcs-commit.toml"),
    include_str!("../keymaps/macos/vcs-diff.toml"),
    include_str!("../keymaps/macos/vcs-push.toml"),
    include_str!("../keymaps/macos/completion.toml"),
    include_str!("../keymaps/macos/navigation.toml"),
    include_str!("../keymaps/macos/find.toml"),
    include_str!("../keymaps/macos/find-in-files.toml"),
    include_str!("../keymaps/macos/search-everywhere.toml"),
    include_str!("../keymaps/macos/console.toml"),
    include_str!("../keymaps/macos/run-configurations.toml"),
    include_str!("../keymaps/macos/project.toml"),
    include_str!("../keymaps/macos/welcome.toml"),
    include_str!("../keymaps/macos/settings.toml"),
    include_str!("../keymaps/macos/breakpoints.toml"),
    include_str!("../keymaps/macos/terminal.toml"),
);
/// The default keymap of Windows and Linux: the files of `keymaps/default/`,
/// one per area, joined in this order (see the module documentation).
pub const DEFAULT: &str = concat!(
    include_str!("../keymaps/default/editor.toml"),
    include_str!("../keymaps/default/code-insight.toml"),
    include_str!("../keymaps/default/window.toml"),
    include_str!("../keymaps/default/run.toml"),
    include_str!("../keymaps/default/debug.toml"),
    include_str!("../keymaps/default/vcs-markers.toml"),
    include_str!("../keymaps/default/vcs-branches.toml"),
    include_str!("../keymaps/default/vcs-commit.toml"),
    include_str!("../keymaps/default/vcs-diff.toml"),
    include_str!("../keymaps/default/vcs-push.toml"),
    include_str!("../keymaps/default/completion.toml"),
    include_str!("../keymaps/default/navigation.toml"),
    include_str!("../keymaps/default/find.toml"),
    include_str!("../keymaps/default/find-in-files.toml"),
    include_str!("../keymaps/default/search-everywhere.toml"),
    include_str!("../keymaps/default/console.toml"),
    include_str!("../keymaps/default/run-configurations.toml"),
    include_str!("../keymaps/default/project.toml"),
    include_str!("../keymaps/default/welcome.toml"),
    include_str!("../keymaps/default/settings.toml"),
    include_str!("../keymaps/default/breakpoints.toml"),
    include_str!("../keymaps/default/terminal.toml"),
);

/// The key bindings of a module (ADR 0022): its area files for macOS and
/// for Windows and Linux, each folder joined in order with `concat!` as
/// [`MACOS`] and [`DEFAULT`] are. They are joined after the workbench's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeymapFiles {
    pub macos: &'static str,
    pub default: &'static str,
}

impl KeymapFiles {
    /// The files of one of the two keymaps.
    pub fn text(&self, macos: bool) -> &'static str {
        if macos { self.macos } else { self.default }
    }
}

/// A default keymap with the bindings of `modules` joined after it.
pub fn with_modules(macos: bool, modules: &[KeymapFiles]) -> String {
    let mut text = String::from(if macos { MACOS } else { DEFAULT });
    for files in modules {
        text.push_str(files.text(macos));
    }
    text
}

/// Checks that every file of a keymap folder is joined into `joined` once,
/// that each ends with a line break and that nothing else is joined: a
/// file missing from a `concat!` list, or in it twice, is an error.
#[cfg(any(test, feature = "test-support"))]
pub fn check_joined_once(folder: &std::path::Path, joined: &str) -> Result<(), String> {
    let entries = std::fs::read_dir(folder).map_err(|e| format!("{folder:?}: {e}"))?;
    let mut length = 0;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            return Err(format!("{path:?} is not a .toml file"));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
        if !text.ends_with('\n') {
            return Err(format!("{path:?} does not end with a line break"));
        }
        let count = joined.matches(&text).count();
        if count != 1 {
            return Err(format!("{path:?} is joined {count} times, not once"));
        }
        length += text.len();
    }
    if joined.len() != length {
        return Err(format!("{folder:?}: something else is joined"));
    }
    Ok(())
}

/// The names of the predefined keymaps, as the reference shows them
/// (`DefaultKeymap.getKeymapPresentableName`).
pub const MACOS_NAME: &str = "macOS";
pub const WINDOWS_NAME: &str = "Windows";
pub const VSCODE_NAME: &str = "VSCode";
pub const VSCODE_MACOS_NAME: &str = "VSCode (macOS)";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default, rename = "context")]
    contexts: Vec<Section>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Section {
    name: Option<String>,
    /// With their places in the file, to keep its order: a command's
    /// first binding is the one shown for it.
    #[serde(default)]
    bindings: BTreeMap<String, toml::Spanned<Target>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Target {
    Command(String),
    WithArgs { command: String, args: toml::Value },
}

/// One keystroke binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    /// Keystrokes, space-separated for sequences.
    pub keys: String,
    pub command: String,
    pub args: Option<Json>,
    /// Key context predicate; `None` binds everywhere.
    pub context: Option<String>,
}

/// A parsed keymap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

impl Keymap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut bindings = Vec::new();
        for section in file.contexts {
            let mut entries: Vec<_> = section.bindings.into_iter().collect();
            entries.sort_by_key(|(_, target)| target.span().start);
            for (keys, target) in entries {
                let (command, args) = match target.into_inner() {
                    Target::Command(command) => (command, None),
                    Target::WithArgs { command, args } => {
                        let args = serde_json::to_value(args).map_err(|e| e.to_string())?;
                        (command, Some(args))
                    }
                };
                bindings.push(Binding {
                    keys,
                    command,
                    args,
                    context: section.name.clone(),
                });
            }
        }
        Ok(Self { bindings })
    }

    /// The default keymap of the platform the IDE runs on.
    pub fn platform_default() -> Self {
        let text = if cfg!(target_os = "macos") {
            MACOS
        } else {
            DEFAULT
        };
        Self::parse(text).expect("the default keymap parses")
    }

    /// Commands bound to a keystroke text, for display (first binding).
    pub fn keys_for(&self, command: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|b| b.command == command && b.args.is_none())
            .map(|b| b.keys.as_str())
    }
}

/// Installs a keymap. Returns the bindings that could not be installed
/// (unknown command, bad arguments, bad keystroke or context).
pub fn bind(cx: &mut App, keymap: &Keymap) -> Vec<String> {
    let mut errors = Vec::new();
    let mut out = Vec::new();
    let mut taps = Vec::new();
    let mut buttons = Vec::new();
    for b in &keymap.bindings {
        let action = match cx.build_action(&b.command, b.args.clone()) {
            Ok(action) => action,
            Err(e) => {
                errors.push(format!("{} -> {}: {e}", b.keys, b.command));
                continue;
            }
        };
        if let Some(key) = parse_double_tap(&b.keys) {
            taps.push((key, b.command.clone(), action));
            continue;
        }
        if let Some(button) = parse_mouse_button(&b.keys) {
            buttons.push((button, action));
            continue;
        }
        let predicate = match &b.context {
            Some(context) => match KeyBindingContextPredicate::parse(context) {
                Ok(p) => Some(Rc::new(p)),
                Err(e) => {
                    errors.push(format!("context `{context}`: {e}"));
                    continue;
                }
            },
            None => None,
        };
        match KeyBinding::load(
            &b.keys,
            action,
            predicate,
            false,
            None,
            &DummyKeyboardMapper,
        ) {
            Ok(binding) => out.push(binding),
            Err(e) => errors.push(format!("{}: {e}", b.keys)),
        }
    }
    cx.bind_keys(out);
    cx.set_global(Gestures {
        taps,
        state: DoubleTap::default(),
        buttons,
    });
    errors
}

/// The mouse button a keymap entry names: `button4` (back) or `button5`
/// (forward).
pub fn parse_mouse_button(keys: &str) -> Option<MouseButton> {
    match keys {
        "button4" => Some(MouseButton::Navigate(NavigationDirection::Back)),
        "button5" => Some(MouseButton::Navigate(NavigationDirection::Forward)),
        _ => None,
    }
}

/// The double-tap and mouse button bindings of the installed keymap and
/// the tap in progress.
#[derive(Default)]
struct Gestures {
    taps: Vec<(ModifierKey, String, Box<dyn Action>)>,
    state: DoubleTap,
    buttons: Vec<(MouseButton, Box<dyn Action>)>,
}

impl Global for Gestures {}

/// The modifiers changed in a window: a completed double tap runs its
/// command there.
pub fn modifiers_changed(modifiers: Modifiers, window: &mut Window, cx: &mut App) {
    let now = cx.background_executor().now();
    let Some(gestures) = cx.try_global::<Gestures>() else {
        return;
    };
    if gestures.taps.is_empty() {
        return;
    }
    let tapped = cx
        .global_mut::<Gestures>()
        .state
        .modifiers_changed(modifiers, now);
    let action = tapped.and_then(|key| {
        cx.global::<Gestures>()
            .taps
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, _, action)| action.boxed_clone())
    });
    if let Some(action) = action {
        window.dispatch_action(action, cx);
    }
}

/// A mouse button went down in a window: a bound button runs its command
/// there. Returns whether one did.
pub fn mouse_down(button: MouseButton, window: &mut Window, cx: &mut App) -> bool {
    let action = cx.try_global::<Gestures>().and_then(|g| {
        g.buttons
            .iter()
            .find(|(b, _)| *b == button)
            .map(|(_, action)| action.boxed_clone())
    });
    match action {
        Some(action) => {
            window.dispatch_action(action, cx);
            true
        }
        None => false,
    }
}

/// A key went down: a double tap in progress is over (a modifier on its
/// own is not a key here).
pub fn key_pressed(keystroke: &Keystroke, cx: &mut App) {
    if matches!(
        keystroke.key.as_str(),
        "shift" | "control" | "alt" | "platform" | "function"
    ) {
        return;
    }
    let now = cx.background_executor().now();
    let guard = !matches!(keystroke.key.as_str(), "escape" | "tab");
    if cx.has_global::<Gestures>() {
        cx.global_mut::<Gestures>().state.key_pressed(now, guard);
    }
}

/// The modifier whose double tap runs `command`, if one does.
pub fn double_tap_for(command: &str, cx: &App) -> Option<ModifierKey> {
    cx.try_global::<Gestures>()?
        .taps
        .iter()
        .find(|(_, c, _)| c == command)
        .map(|(k, _, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contexts_commands_and_arguments() {
        let keymap = Keymap::parse(
            r#"
[[context]]
[context.bindings]
"cmd-q" = "app.quit"

[[context]]
name = "Editor"
[context.bindings]
"cmd-z" = "editor.undo"
"cmd-alt-1" = { command = "tabs.select", args = { index = 0 } }
"#,
        )
        .unwrap();
        assert_eq!(keymap.bindings.len(), 3);
        assert_eq!(keymap.bindings[0].context, None);
        let select = keymap
            .bindings
            .iter()
            .find(|b| b.command == "tabs.select")
            .unwrap();
        assert_eq!(select.args, Some(serde_json::json!({"index": 0})));
        assert_eq!(select.context.as_deref(), Some("Editor"));
        assert_eq!(keymap.keys_for("editor.undo"), Some("cmd-z"));
        assert!(Keymap::parse("[[context]]\nnope = 1\n").is_err());
    }

    #[test]
    fn bindings_keep_the_order_of_the_file() {
        let keymap = Keymap::parse(
            r#"
[[context]]
name = "Editor"
[context.bindings]
"cmd-shift-z" = "editor.redo"
"alt-shift-backspace" = "editor.redo"
"cmd-c" = { command = "editor.copy", args = {} }
"#,
        )
        .unwrap();
        let keys: Vec<&str> = keymap.bindings.iter().map(|b| b.keys.as_str()).collect();
        assert_eq!(keys, ["cmd-shift-z", "alt-shift-backspace", "cmd-c"]);
        assert_eq!(keymap.keys_for("editor.redo"), Some("cmd-shift-z"));
    }

    #[test]
    fn mouse_buttons() {
        let back = MouseButton::Navigate(NavigationDirection::Back);
        let forward = MouseButton::Navigate(NavigationDirection::Forward);
        assert_eq!(parse_mouse_button("button4"), Some(back));
        assert_eq!(parse_mouse_button("button5"), Some(forward));
        assert_eq!(parse_mouse_button("cmd-4"), None);
    }

    #[test]
    fn module_bindings_come_after_the_workbench_ones() {
        let module = KeymapFiles {
            macos: "[[context]]\n[context.bindings]\n\"cmd-alt-f9\" = \"m.run\"\n",
            default: "[[context]]\n[context.bindings]\n\"ctrl-alt-f9\" = \"m.run\"\n",
        };
        for macos in [true, false] {
            let keymap = Keymap::parse(&with_modules(macos, &[module])).unwrap();
            let last = keymap.bindings.last().unwrap();
            assert_eq!(last.command, "m.run");
            let base = Keymap::parse(if macos { MACOS } else { DEFAULT }).unwrap();
            assert_eq!(keymap.bindings.len(), base.bindings.len() + 1);
        }
    }

    #[test]
    fn default_keymaps_parse() {
        for text in [MACOS, DEFAULT] {
            let keymap = Keymap::parse(text).unwrap();
            assert!(keymap.bindings.len() > 50);
        }
    }

    /// A file in a keymap folder that is not in the `concat!` list, or in it
    /// twice, or a list entry that is not a file, fails here.
    #[test]
    fn every_file_of_a_keymap_folder_is_joined_once() {
        let keymaps = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("keymaps");
        for (folder, joined) in [
            ("macos", MACOS),
            ("default", DEFAULT),
            ("actions", actions::ACTIONS),
        ] {
            check_joined_once(&keymaps.join(folder), joined).unwrap();
        }
        // A file twice, or something else joined.
        let macos = keymaps.join("macos");
        let twice = format!("{MACOS}{}", include_str!("../keymaps/macos/editor.toml"));
        assert!(check_joined_once(&macos, &twice).is_err());
        assert!(check_joined_once(&macos, &format!("{MACOS}\n")).is_err());
    }
}
