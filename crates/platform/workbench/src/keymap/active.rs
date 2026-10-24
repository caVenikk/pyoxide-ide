//! The keymap in use (the reference's `KeymapManager` with
//! `QuickChangeKeymapAction`, ADR 0018): the keymaps the IDE offers
//! (predefined and custom), switching (GPUI's bindings, double taps and
//! mouse buttons are replaced and the menus follow), the rows of the Keymap
//! popup, and import of the reference's keymap files.
//!
//! The keymap in use is `[keymap] name` of the settings, else the
//! platform's default ("macOS" on macOS, "Windows" elsewhere). Custom
//! keymaps are derived keymaps in `keymaps/<name>.toml` of the
//! configuration folder. The bindings of the product's modules
//! ([`set_modules`]) are joined after the workbench's in every predefined
//! keymap, before a derived keymap's changes.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use gpui::{App, Global, PathPromptOptions};
use serde_json::{Value as Json, json};
use settings::KeymapSettings;
use settings_ui::commands::OpenSettings;
use ui::icons::IconName;
use ui::menu::MenuItem;
use ui::notification::{Notification, NotificationKind};

use super::actions::ActionTable;
use super::derived::Derived;
use super::reference::{self, Converted, ReferenceKeymap, Unmapped};
use super::{
    DEFAULT, Keymap, KeymapFiles, MACOS, MACOS_NAME, VSCODE_MACOS_NAME, VSCODE_NAME, WINDOWS_NAME,
    bind,
};

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Switch the keymap.
    SwitchKeymap { name: Option<String> } => "settings.switch-keymap", "Keymap",
        "Switches to a keymap by name at once (`macOS`, `Windows`, `VSCode`, `VSCode (macOS)` or an imported one), as the gear menu's Keymap popup does: it is stored, the key bindings, double taps and mouse buttons change, and menus and tooltips show the new shortcuts. Without a name, shows the Keymap popup: the keymaps with the current one marked, Configure Keymap… and Import Keymap….",
        r#"{"name": "VSCode"}"#;
    /// Import a keymap of the reference.
    ImportKeymap { path: Option<String> } => "settings.import-keymap", "Import Keymap\u{2026}",
        "Imports a PyCharm keymap file (`keymaps/<name>.xml` of its configuration folder; a parent keymap next to it is imported first) into the `keymaps` folder of the configuration and switches to it; a balloon lists the shortcuts of actions Pyoxide does not have. Without a path a file chooser is shown.",
        r#"{"path": "/path/to/keymaps/macOS copy.xml"}"#;
}

/// The Keymap popup's title and its row in the Switch popup
/// (`action.ChangeKeymap.text`).
pub const KEYMAP: &str = "Keymap";
/// The gear menu's row (the gear adds an ellipsis).
pub const KEYMAP_IN_GEAR: &str = "Keymap\u{2026}";
/// The Keymap popup's last rows (`keymap.action.configure.keymap`; Import
/// Keymap… in place of `keymap.action.install.keymap`).
pub const CONFIGURE_KEYMAP: &str = "Configure Keymap\u{2026}";
pub const IMPORT_KEYMAP: &str = "Import Keymap\u{2026}";

/// Sets the application menus again (their shortcuts).
type RefreshMenus = Rc<dyn Fn(&mut App)>;

/// Shows a balloon in a main window.
pub type Notify = Rc<dyn Fn(Notification, &mut App)>;

/// A keymap the IDE offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapEntry {
    pub name: String,
    /// Predefined (read-only, bold in the combo box) or custom.
    pub predefined: bool,
    /// The keymap a custom one is based on.
    pub parent: Option<String>,
}

struct State {
    custom: Vec<Derived>,
    /// The bindings of the product's modules.
    modules: Vec<KeymapFiles>,
    /// The folder of custom keymaps.
    dir: Option<PathBuf>,
    /// The keymap whose bindings are installed.
    active: String,
    refresh_menus: Option<RefreshMenus>,
    notify: Option<Notify>,
}

impl Global for State {}

fn state(cx: &mut App) -> &mut State {
    if !cx.has_global::<State>() {
        cx.set_global(State {
            custom: Vec::new(),
            modules: Vec::new(),
            dir: None,
            active: String::new(),
            refresh_menus: None,
            notify: None,
        });
    }
    cx.global_mut::<State>()
}

/// The keymap of the platform the IDE runs on.
pub fn platform_default() -> &'static str {
    if cfg!(target_os = "macos") {
        MACOS_NAME
    } else {
        WINDOWS_NAME
    }
}

/// The predefined keymaps every platform can switch to.
const PREDEFINED: [&str; 4] = [MACOS_NAME, WINDOWS_NAME, VSCODE_NAME, VSCODE_MACOS_NAME];

/// The predefined keymaps the lists offer, in the reference's order: the
/// platform's default first, then by name (`keymapComparator`). Off macOS
/// the macOS keymaps are not offered (`DefaultKeymap.isBundledKeymapHidden`).
fn offered() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec![MACOS_NAME, VSCODE_NAME, VSCODE_MACOS_NAME, WINDOWS_NAME]
    } else {
        vec![WINDOWS_NAME, VSCODE_NAME]
    }
}

/// Registers the commands and the `[keymap]` section, follows the stored
/// keymap, and installs the platform's default keymap.
pub fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    settings_ui::store::register::<KeymapSettings>(cx);
    state(cx);
    cx.on_action(|a: &SwitchKeymap, cx| {
        if let Some(name) = &a.name
            && let Err(e) = switch(name, cx)
        {
            eprintln!("settings.switch-keymap: {e}");
        }
    });
    cx.on_action(|a: &ImportKeymap, cx| match &a.path {
        Some(path) => import(PathBuf::from(path), cx),
        None => choose_and_import(cx),
    });
    settings_ui::store::observe(cx, <KeymapSettings as settings::Section>::KEY, follow_store);
    if let Err(e) = activate(platform_default(), cx) {
        eprintln!("keymap: {e}");
    }
}

/// The key bindings of the product's modules, joined after the
/// workbench's in every predefined keymap; set before [`init`].
pub fn set_modules(cx: &mut App, modules: Vec<KeymapFiles>) {
    state(cx).modules = modules;
}

/// Reads the custom keymaps of `dir` (start-up, before the first window)
/// and installs the stored keymap.
pub fn load(cx: &mut App, dir: Option<PathBuf>) {
    let custom = match &dir {
        Some(dir) => read_dir(dir),
        None => Vec::new(),
    };
    let s = state(cx);
    s.custom = custom;
    s.dir = dir;
    follow_store(cx);
}

fn read_dir(dir: &Path) -> Vec<Derived> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    let mut out: Vec<Derived> = Vec::new();
    for path in paths {
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| Derived::parse(&text));
        match parsed {
            Ok(k) if PREDEFINED.contains(&k.name.as_str()) => {
                eprintln!("keymaps: {}: a predefined keymap's name", path.display())
            }
            Ok(k) if out.iter().any(|o| o.name == k.name) => {
                eprintln!("keymaps: {}: `{}` again", path.display(), k.name)
            }
            Ok(k) => out.push(k),
            Err(e) => eprintln!("keymaps: {}: {e}", path.display()),
        }
    }
    out
}

/// Sets the hook that sets the application menus again after a switch.
pub fn set_menus(cx: &mut App, refresh: RefreshMenus) {
    state(cx).refresh_menus = Some(refresh);
}

/// Where balloons about imports go (the workbench's main windows).
pub fn set_notify(cx: &mut App, notify: Notify) {
    state(cx).notify = Some(notify);
}

/// The keymap whose bindings are installed.
pub fn active_name(cx: &App) -> String {
    cx.try_global::<State>()
        .map(|s| s.active.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| platform_default().to_owned())
}

fn custom(cx: &App) -> &[Derived] {
    cx.try_global::<State>()
        .map(|s| s.custom.as_slice())
        .unwrap_or_default()
}

/// A keymap of this name exists (offered or not).
pub fn is_known(name: &str, cx: &App) -> bool {
    PREDEFINED.contains(&name) || custom(cx).iter().any(|k| k.name == name)
}

/// The keymaps the combo box and the Keymap popup offer, in the
/// reference's order: each predefined keymap followed by the custom ones
/// based on it (by name), then custom ones based on keymaps not offered.
pub fn entries(cx: &App) -> Vec<KeymapEntry> {
    let custom = custom(cx);
    let mut out = Vec::new();
    let mut listed = vec![false; custom.len()];
    let root = |k: &Derived| -> String {
        let mut name = k.parent.clone();
        for _ in 0..8 {
            match custom.iter().find(|c| c.name == name) {
                Some(parent) => name = parent.parent.clone(),
                None => break,
            }
        }
        name
    };
    let mut by_name: Vec<usize> = (0..custom.len()).collect();
    by_name.sort_by(|&a, &b| custom[a].name.cmp(&custom[b].name));
    for predefined in offered() {
        out.push(KeymapEntry {
            name: predefined.to_owned(),
            predefined: true,
            parent: None,
        });
        for &ix in &by_name {
            if !listed[ix] && root(&custom[ix]) == predefined {
                listed[ix] = true;
                out.push(KeymapEntry {
                    name: custom[ix].name.clone(),
                    predefined: false,
                    parent: Some(custom[ix].parent.clone()),
                });
            }
        }
    }
    for &ix in &by_name {
        if !listed[ix] {
            out.push(KeymapEntry {
                name: custom[ix].name.clone(),
                predefined: false,
                parent: Some(custom[ix].parent.clone()),
            });
        }
    }
    out
}

/// The bindings of a keymap by name.
pub fn resolve(name: &str, cx: &App) -> Result<Keymap, String> {
    let modules = cx.try_global::<State>().map_or(&[][..], |s| &s.modules);
    resolve_in(name, custom(cx), modules)
}

/// The bindings of a keymap by name, with `custom` as the custom keymaps.
pub fn resolve_with(name: &str, custom: &[Derived]) -> Result<Keymap, String> {
    resolve_in(name, custom, &[])
}

/// The bindings of a keymap by name, with `custom` as the custom keymaps
/// and the bindings of `modules` in the predefined ones.
pub fn resolve_in(
    name: &str,
    custom: &[Derived],
    modules: &[KeymapFiles],
) -> Result<Keymap, String> {
    let mut chain: Vec<&Derived> = Vec::new();
    let mut current = name;
    loop {
        if let Some(base) = predefined_with(current, modules) {
            let mut keymap = base?;
            for derived in chain.iter().rev() {
                keymap = derived.apply(&keymap);
            }
            return Ok(keymap);
        }
        let derived = custom
            .iter()
            .find(|k| k.name == current)
            .ok_or_else(|| format!("unknown keymap `{current}`"))?;
        if chain.len() > 8 || chain.iter().any(|k| k.name == derived.name) {
            return Err(format!("the parents of keymap `{name}` form a loop"));
        }
        chain.push(derived);
        current = &derived.parent;
    }
}

/// A predefined keymap's bindings with the modules' joined after the
/// workbench's; a VS Code keymap's changes apply to both.
fn predefined_with(name: &str, modules: &[KeymapFiles]) -> Option<Result<Keymap, String>> {
    if modules.is_empty() {
        return predefined(name);
    }
    let (macos, vscode) = match name {
        MACOS_NAME => (true, false),
        WINDOWS_NAME => (false, false),
        VSCODE_NAME => (false, true),
        VSCODE_MACOS_NAME => (true, true),
        _ => return None,
    };
    let base = predefined(if macos { MACOS_NAME } else { WINDOWS_NAME })?;
    Some(base.and_then(|mut keymap| {
        for files in modules {
            let text = if macos { files.macos } else { files.default };
            keymap.bindings.extend(Keymap::parse(text)?.bindings);
        }
        Ok(match vscode {
            true => vscode_changes(macos).apply(&keymap),
            false => keymap,
        })
    }))
}

/// The changes of the reference's VS Code keymap (converted once).
fn vscode_changes(macos: bool) -> &'static Derived {
    static VSCODE: OnceLock<Derived> = OnceLock::new();
    static VSCODE_MAC: OnceLock<Derived> = OnceLock::new();
    let cell = if macos { &VSCODE_MAC } else { &VSCODE };
    cell.get_or_init(|| vscode(macos).derived)
}

/// A predefined keymap's bindings (parsed once).
fn predefined(name: &str) -> Option<Result<Keymap, String>> {
    static MAC: OnceLock<Result<Keymap, String>> = OnceLock::new();
    static WIN: OnceLock<Result<Keymap, String>> = OnceLock::new();
    static VSCODE: OnceLock<Result<Keymap, String>> = OnceLock::new();
    static VSCODE_MAC: OnceLock<Result<Keymap, String>> = OnceLock::new();
    let cell = match name {
        MACOS_NAME => MAC.get_or_init(|| Keymap::parse(MACOS)),
        WINDOWS_NAME => WIN.get_or_init(|| Keymap::parse(DEFAULT)),
        VSCODE_NAME => VSCODE.get_or_init(|| {
            let parent = Keymap::parse(DEFAULT)?;
            Ok(vscode(false).derived.apply(&parent))
        }),
        VSCODE_MACOS_NAME => VSCODE_MAC.get_or_init(|| {
            let parent = Keymap::parse(MACOS)?;
            Ok(vscode(true).derived.apply(&parent))
        }),
        _ => return None,
    };
    Some(cell.clone())
}

/// The reference's VS Code keymap (for macOS or not) converted through the
/// action table.
pub fn vscode(macos: bool) -> Converted {
    let (xml, name, parent) = if macos {
        (reference::VSCODE_MACOS, VSCODE_MACOS_NAME, MACOS_NAME)
    } else {
        (reference::VSCODE, VSCODE_NAME, WINDOWS_NAME)
    };
    let keymap = ReferenceKeymap::parse(xml).expect("the VS Code keymaps parse");
    reference::convert(&keymap, name, parent, ActionTable::builtin())
}

/// The keymap the settings name, else the platform's default.
fn stored(cx: &App) -> String {
    let stored: KeymapSettings = settings_ui::store::get(cx);
    stored
        .name
        .filter(|n| is_known(n, cx))
        .unwrap_or_else(|| platform_default().to_owned())
}

fn follow_store(cx: &mut App) {
    let wanted = stored(cx);
    if wanted != active_name(cx)
        && let Err(e) = activate(&wanted, cx)
    {
        eprintln!("keymap: {e}");
    }
}

/// Installs a keymap's bindings in place of the current ones (not stored).
pub fn activate(name: &str, cx: &mut App) -> Result<(), String> {
    let keymap = resolve(name, cx)?;
    cx.clear_key_bindings();
    for error in bind(cx, &keymap) {
        eprintln!("keymap {name}: {error}");
    }
    state(cx).active = name.to_owned();
    if let Some(refresh) = state(cx).refresh_menus.clone() {
        refresh(cx);
    }
    Ok(())
}

/// Switches to a keymap at once and stores it.
pub fn switch(name: &str, cx: &mut App) -> Result<(), String> {
    if !is_known(name, cx) {
        return Err(format!("unknown keymap `{name}`"));
    }
    let value = KeymapSettings {
        name: (name != platform_default()).then(|| name.to_owned()),
    };
    settings_ui::store::set(cx, &value);
    // Stored already (the store does not tell): install it anyway.
    if active_name(cx) != name {
        activate(name, cx)?;
    }
    Ok(())
}

/// The rows of the Keymap popup (`QuickChangeKeymapAction`): the keymaps
/// with an arrow at the current one, then Configure Keymap… and Import
/// Keymap…; and the current one's row.
pub fn popup_items(cx: &App) -> (Vec<MenuItem>, Option<usize>) {
    let current = active_name(cx);
    let mut items = Vec::new();
    let mut selected = None;
    for entry in entries(cx) {
        let mut item = MenuItem::action(
            entry.name.clone(),
            SwitchKeymap {
                name: Some(entry.name.clone()),
            },
        );
        if entry.name == current {
            selected = Some(items.len());
            item = item.icon(IconName::FORWARD);
        }
        items.push(item);
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action(
        CONFIGURE_KEYMAP,
        OpenSettings {
            page: Some(crate::settings::KEYMAP_PAGE.to_owned()),
            filter: None,
        },
    ));
    items.push(MenuItem::action(IMPORT_KEYMAP, ImportKeymap { path: None }));
    (items, selected)
}

/// The keymaps for `workbench.state`.
pub fn state_json(cx: &App) -> Json {
    json!({
        "name": active_name(cx),
        "keymaps": entries(cx).iter().map(|e| json!({
            "name": e.name,
            "predefined": e.predefined,
            "parent": e.parent,
        })).collect::<Vec<_>>(),
    })
}

// ---- import --------------------------------------------------------------

/// Shows the file chooser and imports the file chosen.
pub fn choose_and_import(cx: &mut App) {
    let chosen = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = chosen.await
            && let Some(path) = paths.into_iter().next()
        {
            let _ = cx.update(|cx| import(path, cx));
        }
    })
    .detach();
}

/// What an import made.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The keymaps written, parents first; the last is the one asked for.
    pub keymaps: Vec<Derived>,
    /// Shortcuts of actions Pyoxide does not have, of every keymap.
    pub unmapped: Vec<Unmapped>,
    /// Parents Pyoxide only approximates: (the reference's name, the
    /// keymap used in its place).
    pub approximated: Vec<(String, String)>,
}

/// Imports a keymap file of the reference in the background into the
/// folder of custom keymaps and switches to it; a balloon tells what
/// happened.
pub fn import(path: PathBuf, cx: &mut App) {
    let Some(dir) = state(cx).dir.clone() else {
        report(
            Notification::new(
                NotificationKind::Error,
                "Keymap import failed",
                "There is no configuration folder to keep imported keymaps in.",
            ),
            cx,
        );
        return;
    };
    let known: Vec<String> = state(cx).custom.iter().map(|k| k.name.clone()).collect();
    let work = cx
        .background_executor()
        .spawn(async move { import_into(&path, &dir, &known) });
    cx.spawn(async move |cx| {
        let result = work.await;
        let _ = cx.update(|cx| imported(result, cx));
    })
    .detach();
}

/// The file name of a custom keymap: its name with characters that do not
/// belong in file names replaced.
pub fn file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " ._-()+".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.toml", safe.trim())
}

/// Reads `path` (and the parent keymaps next to it that are neither
/// predefined nor in `known`), converts them and writes them into `dir`.
pub fn import_into(path: &Path, dir: &Path, known: &[String]) -> Result<Imported, String> {
    let read = |path: &Path| -> Result<ReferenceKeymap, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        ReferenceKeymap::parse(&text)
            .map_err(|e| format!("{} is not a keymap: {e}", path.display()))
    };
    // The file and its parents next to it, the file first.
    let mut chain = vec![read(path)?];
    let mut current = path.to_path_buf();
    while chain.len() < 8 {
        let Some(parent) = chain.last().and_then(|k| k.parent.clone()) else {
            break;
        };
        if reference::builtin_parent(&parent).is_some()
            || known.contains(&parent)
            || chain.iter().any(|k| k.name == parent)
        {
            break;
        }
        let sibling = current.with_file_name(format!("{parent}.xml"));
        if !sibling.is_file() {
            break;
        }
        chain.push(read(&sibling)?);
        current = sibling;
    }
    let mut imported = Imported {
        keymaps: Vec::new(),
        unmapped: Vec::new(),
        approximated: Vec::new(),
    };
    let mut names: Vec<String> = known.to_vec();
    for keymap in chain.iter().rev() {
        if PREDEFINED.contains(&keymap.name.as_str())
            || reference::builtin_parent(&keymap.name).is_some()
        {
            return Err(format!("{} is a predefined keymap", keymap.name));
        }
        let parent = match keymap.parent.as_deref() {
            Some(p) if names.iter().any(|n| n == p) => p.to_owned(),
            Some(p) => match reference::builtin_parent(p) {
                Some((name, false)) => name.to_owned(),
                Some((name, true)) => {
                    imported.approximated.push((p.to_owned(), name.to_owned()));
                    name.to_owned()
                }
                None => {
                    let fallback = if p.contains("Mac OS X") || p.contains("OSX") {
                        MACOS_NAME
                    } else {
                        platform_default()
                    };
                    imported
                        .approximated
                        .push((p.to_owned(), fallback.to_owned()));
                    fallback.to_owned()
                }
            },
            None => {
                imported
                    .approximated
                    .push((String::new(), platform_default().to_owned()));
                platform_default().to_owned()
            }
        };
        let converted = reference::convert(keymap, &keymap.name, &parent, ActionTable::builtin());
        imported.unmapped.extend(converted.unmapped);
        names.push(keymap.name.clone());
        imported.keymaps.push(converted.derived);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    for keymap in &imported.keymaps {
        let file = dir.join(file_name(&keymap.name));
        settings::write_atomic(&file, &keymap.to_toml())
            .map_err(|e| format!("Cannot write {}: {e}", file.display()))?;
    }
    Ok(imported)
}

fn imported(result: Result<Imported, String>, cx: &mut App) {
    let imported = match result {
        Ok(imported) => imported,
        Err(e) => {
            report(
                Notification::new(NotificationKind::Error, "Keymap import failed", e),
                cx,
            );
            return;
        }
    };
    if let Some(name) = add_imported(&imported, cx) {
        report(import_notification(&name, &imported), cx);
    }
}

/// Adds the keymaps an import wrote ([`import_into`]) and switches to the
/// last of them, without a balloon; returns its name.
pub fn add_imported(imported: &Imported, cx: &mut App) -> Option<String> {
    for keymap in &imported.keymaps {
        let custom = &mut state(cx).custom;
        custom.retain(|k| k.name != keymap.name);
        custom.push(keymap.clone());
    }
    let name = imported.keymaps.last().map(|k| k.name.clone())?;
    // The keymap's bindings changed even when its name did not.
    if active_name(cx) == name {
        state(cx).active.clear();
    }
    if let Err(e) = switch(&name, cx) {
        eprintln!("keymaps: {e}");
    }
    cx.refresh_windows();
    Some(name)
}

/// The balloon of a finished import.
pub fn import_notification(name: &str, imported: &Imported) -> Notification {
    let parent = imported
        .keymaps
        .last()
        .map(|k| k.parent.clone())
        .unwrap_or_default();
    let mut text = format!("{name} is now the keymap, based on {parent}.");
    for (reference, used) in &imported.approximated {
        if reference.is_empty() {
            text.push_str(&format!(" It names no parent keymap; {used} is used."));
        } else {
            text.push_str(&format!(
                " Pyoxide has no {reference} keymap; {used} is used."
            ));
        }
    }
    let unmapped = &imported.unmapped;
    if !unmapped.is_empty() {
        let shown: Vec<String> = unmapped
            .iter()
            .take(5)
            .map(|u| format!("{} ({})", u.action, u.shortcuts.join(", ")))
            .collect();
        let more = unmapped.len() - shown.len();
        text.push_str(&format!(
            " Shortcuts of actions Pyoxide does not have: {}{}.",
            shown.join("; "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ));
    }
    let kind = if imported.approximated.is_empty() {
        NotificationKind::Information
    } else {
        NotificationKind::Warning
    };
    Notification::new(kind, "Keymap imported", text)
}

fn report(notification: Notification, cx: &mut App) {
    match state(cx).notify.clone() {
        Some(notify) => notify(notification, cx),
        None => eprintln!("{}: {}", notification.title, notification.content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_of_custom_keymaps() {
        assert_eq!(file_name("macOS copy"), "macOS copy.toml");
        assert_eq!(file_name("a/b:c"), "a_b_c.toml");
    }

    #[test]
    fn custom_keymaps_resolve_through_their_parents() {
        let base = Derived::parse(
            "version = 1\nname = \"Mine\"\nparent = \"macOS\"\n[[override]]\ncommand = \"editor.duplicate\"\ncontext = \"Editor\"\nkeys = [\"ctrl-d\"]\n",
        )
        .unwrap();
        let child = Derived::parse(
            "version = 1\nname = \"Mine 2\"\nparent = \"Mine\"\n[[override]]\ncommand = \"tabs.close\"\nkeys = [\"ctrl-w\"]\n",
        )
        .unwrap();
        let custom = vec![base, child];
        let keymap = resolve_with("Mine 2", &custom).unwrap();
        let keys = |command: &str| -> Vec<&str> {
            keymap
                .bindings
                .iter()
                .filter(|b| b.command == command)
                .map(|b| b.keys.as_str())
                .collect()
        };
        assert_eq!(keys("editor.duplicate"), ["ctrl-d"]);
        assert_eq!(keys("tabs.close"), ["ctrl-w"]);
        assert!(resolve_with("Nope", &custom).is_err());
        let looped = vec![
            Derived::parse("version = 1\nname = \"A\"\nparent = \"B\"\n").unwrap(),
            Derived::parse("version = 1\nname = \"B\"\nparent = \"A\"\n").unwrap(),
        ];
        assert!(resolve_with("A", &looped).unwrap_err().contains("loop"));
    }

    #[test]
    fn module_bindings_reach_custom_keymaps_and_their_changes_win() {
        let module = KeymapFiles {
            macos: "[[context]]\n[context.bindings]\n\"cmd-alt-shift-f9\" = \"tabs.close\"\n",
            default: "[[context]]\n[context.bindings]\n\"ctrl-alt-shift-f9\" = \"tabs.close\"\n",
        };
        let mine = Derived::parse(
            "version = 1\nname = \"Mine\"\nparent = \"macOS\"\n[[override]]\ncommand = \"tabs.close\"\nkeys = [\"ctrl-w\"]\n",
        )
        .unwrap();
        let keys = |keymap: &Keymap| -> Vec<String> {
            keymap
                .bindings
                .iter()
                .filter(|b| b.command == "tabs.close" && b.args.is_none())
                .map(|b| b.keys.clone())
                .collect()
        };
        let mac = resolve_in(MACOS_NAME, &[], &[module]).unwrap();
        assert!(keys(&mac).contains(&"cmd-alt-shift-f9".to_owned()));
        let windows = resolve_in(WINDOWS_NAME, &[], &[module]).unwrap();
        assert!(keys(&windows).contains(&"ctrl-alt-shift-f9".to_owned()));
        // A custom keymap's change replaces the module's binding too.
        let custom = resolve_in("Mine", &[mine], &[module]).unwrap();
        assert_eq!(keys(&custom), ["ctrl-w"]);
        // Without modules: the workbench's keymap as before.
        assert_eq!(
            resolve_in(MACOS_NAME, &[], &[]).unwrap(),
            Keymap::parse(MACOS).unwrap()
        );
    }

    #[test]
    fn imports_a_keymap_with_its_parent_next_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("reference");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(
            source.join("Base.xml"),
            r#"<keymap version="1" name="Base" parent="Mac OS X 10.5+">
  <action id="EditorDuplicate"><keyboard-shortcut first-keystroke="ctrl D"/></action>
</keymap>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Mine.xml"),
            r#"<keymap version="1" name="Mine" parent="Base">
  <action id="CloseContent"><keyboard-shortcut first-keystroke="ctrl W"/></action>
  <action id="Unknown"><keyboard-shortcut first-keystroke="ctrl U"/></action>
</keymap>"#,
        )
        .unwrap();
        let keymaps = dir.path().join("keymaps");
        let imported = import_into(&source.join("Mine.xml"), &keymaps, &[]).unwrap();
        let names: Vec<_> = imported.keymaps.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["Base", "Mine"]);
        assert_eq!(imported.keymaps[0].parent, MACOS_NAME);
        assert_eq!(imported.keymaps[1].parent, "Base");
        assert_eq!(imported.unmapped.len(), 1);
        assert!(imported.approximated.is_empty());
        let written = read_dir(&keymaps);
        assert_eq!(written, imported.keymaps);
        let text = import_notification("Mine", &imported).content.to_string();
        let u = if cfg!(target_os = "macos") {
            "⌃U"
        } else {
            "Ctrl+U"
        };
        assert!(text.contains(&format!("Unknown ({u})")), "{text}");
        // A predefined keymap is not imported.
        std::fs::write(
            source.join("Default.xml"),
            r#"<keymap version="1" name="$default"></keymap>"#,
        )
        .unwrap();
        assert!(import_into(&source.join("Default.xml"), &keymaps, &[]).is_err());
    }
}
