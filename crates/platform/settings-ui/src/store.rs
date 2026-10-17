//! The running application's settings store (`settings::Store` in a GPUI
//! global): typed sections for the areas that read them, change events per
//! section, and saves in the background, one at a time.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Global};
use serde_json::Value as Json;
use settings::{Section, SettingsError, Store};

type Listener = Rc<dyn Fn(&mut App)>;

#[derive(Default)]
struct AppSettings {
    store: Store,
    /// Registrations to repeat on a store loaded later.
    registrations: Vec<fn(&mut Store)>,
    listeners: Vec<(&'static str, Listener)>,
    /// A save runs; the next one starts after it.
    saving: bool,
}

impl Global for AppSettings {}

fn register_in<T: Section>(store: &mut Store) {
    store.register::<T>();
}

/// Makes a section known to the store (its defaults, `settings.get` and
/// `settings.set`); call it at start-up for every section an area reads.
pub fn register<T: Section>(cx: &mut App) {
    let g = cx.default_global::<AppSettings>();
    let f = register_in::<T> as fn(&mut Store);
    if !g.registrations.contains(&f) {
        g.registrations.push(f);
    }
    g.store.register::<T>();
}

/// Reads the settings from `file` (the application's `settings.toml`) and
/// saves them there from now on; every section's listeners hear of it.
pub fn load(cx: &mut App, file: PathBuf) {
    let mut store = Store::load(file);
    if let Some(problem) = store.problem() {
        eprintln!("settings: {problem:?}");
    }
    let g = cx.default_global::<AppSettings>();
    for f in &g.registrations {
        f(&mut store);
    }
    g.store = store;
    let listeners: Vec<Listener> = g.listeners.iter().map(|(_, l)| l.clone()).collect();
    for l in listeners {
        l(cx);
    }
}

/// The settings file the store reads and saves (`None` before [`load`]).
pub fn file(cx: &App) -> Option<PathBuf> {
    cx.try_global::<AppSettings>()?
        .store
        .file()
        .map(|f| f.to_path_buf())
}

/// The section as stored (its defaults without a store).
pub fn get<T: Section>(cx: &App) -> T {
    cx.try_global::<AppSettings>()
        .map(|g| g.store.section::<T>())
        .unwrap_or_default()
}

/// Stores a section; the section's listeners hear of a change and it is
/// saved in the background.
pub fn set<T: Section>(cx: &mut App, value: &T) {
    let changed = cx.default_global::<AppSettings>().store.set_section(value);
    if changed {
        changed_section(cx, T::KEY);
    }
}

/// A setting or a table of them by dotted key (`""`: all of them).
pub fn get_key(cx: &App, key: &str) -> Option<Json> {
    cx.try_global::<AppSettings>()?.store.get(key)
}

/// Changes one setting by dotted key (`settings.set`).
pub fn set_key(cx: &mut App, key: &str, value: Json) -> Result<(), SettingsError> {
    let changed = cx.default_global::<AppSettings>().store.set(key, value)?;
    if let Some(section) = changed {
        changed_section(cx, section);
    }
    Ok(())
}

/// Runs `f` after every change of the section `key` (also when the store
/// is loaded).
pub fn observe(cx: &mut App, key: &'static str, f: impl Fn(&mut App) + 'static) {
    cx.default_global::<AppSettings>()
        .listeners
        .push((key, Rc::new(f)));
}

/// Writes the changes not saved yet now (on quit).
pub fn flush(cx: &mut App) {
    if let Some(g) = cx.try_global::<AppSettings>()
        && g.store.is_dirty()
        && let Err(e) = cx.global_mut::<AppSettings>().store.save_now()
    {
        eprintln!("settings: cannot save: {e}");
    }
}

fn changed_section(cx: &mut App, key: &'static str) {
    let listeners: Vec<Listener> = cx
        .global::<AppSettings>()
        .listeners
        .iter()
        .filter(|(k, _)| *k == key)
        .map(|(_, l)| l.clone())
        .collect();
    for l in listeners {
        l(cx);
    }
    schedule_save(cx);
}

fn schedule_save(cx: &mut App) {
    let g = cx.global_mut::<AppSettings>();
    if g.saving {
        return;
    }
    let Some(plan) = g.store.take_save() else {
        return;
    };
    g.saving = true;
    let write = cx.background_executor().spawn(async move { plan.write() });
    cx.spawn(async move |cx| {
        if let Err(e) = write.await {
            eprintln!("settings: cannot save: {e}");
        }
        let _ = cx.update(|cx| {
            cx.global_mut::<AppSettings>().saving = false;
            schedule_save(cx);
        });
    })
    .detach();
}
