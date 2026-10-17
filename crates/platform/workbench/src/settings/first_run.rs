//! The workbench's part of the first-run window (ADR 0017,
//! `settings_ui::first_run`): the keymaps Choose Keymap offers with their
//! sample shortcuts, the importers of the reference's keymap and of the
//! Search Everywhere preview, the import's balloon in a main window, and
//! `settings.import-reference` over a main window.

use std::rc::Rc;

use gpui::{App, Context, Keystroke, Window};
use settings_import::pycharm::options;
use settings_ui::first_run::{
    self, Applied, ImportReference, Importer, KeymapChoice, Source, Start,
};

use crate::keymap::active::{self as keymaps};
use crate::keymap::{double_tap_text, parse_double_tap, reference};
use crate::workbench::Workbench;

pub(super) fn init(cx: &mut App) {
    first_run::set_keymaps(
        cx,
        KeymapChoice {
            keymaps: offered,
            shortcuts,
            current: |cx| keymaps::active_name(cx),
            switch: keymaps::switch,
        },
    );
    first_run::register_importer(
        cx,
        Importer {
            id: "keymap",
            work: import_keymap,
        },
    );
    first_run::register_importer(
        cx,
        Importer {
            id: "search-everywhere",
            work: import_preview,
        },
    );
    first_run::set_notify(cx, Rc::new(super::notify_main_window));
}

/// The first-run window's start: the product's name, the reference's
/// folders (`PYOXIDE_REFERENCE_CONFIG`, else the platform's).
pub fn start(title: &str) -> Start {
    Start {
        title: title.to_owned().into(),
        reference_root: crate::env::reference_config_root().or_else(options::config_root),
        activate: !crate::env::background(),
    }
}

/// The predefined keymaps the platform offers, then the custom keymap in
/// use (an imported one).
fn offered(cx: &App) -> Vec<String> {
    let current = keymaps::active_name(cx);
    let mut out: Vec<String> = keymaps::entries(cx)
        .into_iter()
        .filter(|e| e.predefined || e.name == current)
        .map(|e| e.name)
        .collect();
    if !out.contains(&current) {
        out.push(current);
    }
    out
}

/// The shortcuts of `commands` in `keymap`, as hints show them (`⇧⌘A`,
/// `Double ⇧`, `Ctrl+Shift+A`).
fn shortcuts(keymap: &str, commands: &[&str], cx: &App) -> Vec<Option<String>> {
    let resolved = keymaps::resolve(keymap, cx).ok();
    let macos = cfg!(target_os = "macos");
    commands
        .iter()
        .map(|command| {
            let keys = resolved.as_ref()?.keys_for(command)?;
            Some(shortcut_text(keys, macos))
        })
        .collect()
}

fn shortcut_text(keys: &str, macos: bool) -> String {
    if let Some(key) = parse_double_tap(keys) {
        return double_tap_text(key, macos);
    }
    keys.split(' ')
        .map(|stroke| match Keystroke::parse(stroke) {
            Ok(k) => ui::keys::format_keystroke(&k, macos),
            Err(_) => stroke.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The reference's keymap: a predefined one by its Pyoxide name, a custom
/// one imported from `keymaps/` (with the custom parents next to it).
fn import_keymap(source: &Source) -> Applied {
    let Some(name) = source.options.keymap.clone() else {
        return first_run::nothing();
    };
    if let Some((ours, approximated)) = reference::builtin_parent(&name) {
        return Box::new(move |cx| {
            let mut outcome = first_run::Outcome::default();
            if let Err(e) = keymaps::switch(ours, cx) {
                outcome.problems.push(format!("{e}."));
                return outcome;
            }
            outcome.imported.push(format!("the keymap {ours}"));
            if approximated {
                outcome
                    .problems
                    .push(format!("Pyoxide has no {name} keymap; {ours} is used."));
            }
            outcome
        });
    }
    let (Some(file), Some(dir)) = (
        options::keymap_file(source.snapshot.dir(), &name),
        source.config_dir.as_ref().map(|d| d.join("keymaps")),
    ) else {
        return Box::new(move |_| {
            first_run::Outcome::problem(format!(
                "The keymap {name} was not found; the default keymap is kept."
            ))
        });
    };
    let result = keymaps::import_into(&file, &dir, &[]);
    Box::new(move |cx| match result {
        Ok(imported) => {
            let Some(name) = keymaps::add_imported(&imported, cx) else {
                return first_run::Outcome::default();
            };
            let parent = imported
                .keymaps
                .last()
                .map(|k| k.parent.clone())
                .unwrap_or_default();
            let mut outcome =
                first_run::Outcome::imported(format!("the keymap {name} (based on {parent})"));
            for (reference, used) in &imported.approximated {
                if !reference.is_empty() {
                    outcome.problems.push(format!(
                        "Pyoxide has no {reference} keymap; {used} is used."
                    ));
                }
            }
            let unmapped: usize = imported.unmapped.iter().map(|u| u.shortcuts.len()).sum();
            if unmapped > 0 {
                outcome.problems.push(format!(
                    "{unmapped} of its shortcuts are for actions Pyoxide does not have."
                ));
            }
            outcome
        }
        Err(e) => first_run::Outcome::problem(format!("The keymap could not be imported: {e}.")),
    })
}

/// "Show preview" of Search Everywhere (`SHOW_PREVIEW_IN_SEARCH_EVERYWHERE`).
fn import_preview(source: &Source) -> Applied {
    let Some(preview) = source.options.ui.search_everywhere_preview else {
        return first_run::nothing();
    };
    Box::new(move |cx| {
        if crate::search_everywhere::settings(cx).preview == preview {
            return first_run::Outcome::default();
        }
        crate::search_everywhere::update_settings(cx, |s| s.preview = preview);
        first_run::Outcome::imported(if preview {
            "the Search Everywhere preview (on)"
        } else {
            "the Search Everywhere preview (off)"
        })
    })
}

impl Workbench {
    /// `settings.import-reference`: the first-run window over this one.
    pub(crate) fn import_reference_settings(
        &mut self,
        _: &ImportReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = self.product_name();
        first_run::open_over(window, start(title), cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_hints() {
        assert_eq!(shortcut_text("shift shift", true), "Double \u{21e7}");
        assert_eq!(shortcut_text("cmd-shift-a", true), "\u{21e7}\u{2318}A");
        assert_eq!(shortcut_text("ctrl-shift-a", false), "Ctrl+Shift+A");
        assert_eq!(shortcut_text("ctrl-k ctrl-s", false), "Ctrl+K, Ctrl+S");
    }
}
