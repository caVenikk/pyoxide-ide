//! The first-run window (tests/scenarios/settings/reference-import.md):
//! with an empty configuration folder it offers the reference's newest
//! configuration folder, imports its theme (from a theme plugin), font,
//! soft wraps, autosave, file types and custom keymap, preselects them on
//! Choose Theme and Choose Keymap and records the decision; Skip Import
//! keeps the defaults; with nothing to import it opens on Choose Theme.

use std::cell::Cell;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui::{AnyWindowHandle, AppContext as _, TestAppContext};
use serde_json::{Value, json};
use settings_ui::first_run::{self, FirstRunWindow, Start};
use settings_ui::theme_switch;
use workbench::Product;

struct H<'a> {
    cx: &'a mut TestAppContext,
    /// `settings.toml`.
    file: PathBuf,
    /// Set when the window closed and the start-up went on.
    done: Rc<Cell<bool>>,
    _dir: tempfile::TempDir,
}

impl H<'_> {
    fn window(&mut self) -> Option<AnyWindowHandle> {
        self.cx.update(|cx| {
            cx.windows()
                .into_iter()
                .find(|w| w.downcast::<FirstRunWindow>().is_some())
        })
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.cx.run_until_parked();
            self.cx.executor().advance_clock(Duration::from_millis(50));
        }
        self.cx.run_until_parked();
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let window = self.window().expect("the first-run window");
        let out = self
            .cx
            .update_window(window, |_, window, cx| {
                window.draw(cx).clear();
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.settle();
        out
    }

    fn state(&mut self) -> Value {
        self.invoke("first-run.state", json!(null))
    }

    fn get(&mut self, key: &str) -> Value {
        self.cx
            .update(|cx| settings_ui::store::get_key(cx, key))
            .unwrap_or(Value::Null)
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn os_folder() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

fn themes_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../themes/tests/fixtures/plugin/themes")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

/// A configuration folder of the reference with a theme plugin, a custom
/// keymap and options; an older one next to it.
fn reference(root: &Path) {
    let dir = root.join("PyCharm2026.2");
    let o = dir.join("options");
    write(&o.join("other.xml"), "<application/>");
    write(
        &o.join("laf.xml"),
        r#"<application><component name="LafManager"><laf themeId="test-nightfall" /></component></application>"#,
    );
    write(
        &o.join("colors.scheme.xml"),
        r#"<application><component name="EditorColorsManagerImpl"><global_color_scheme name="Nightfall" /></component></application>"#,
    );
    write(
        &o.join(os_folder()).join("keymap.xml"),
        r#"<application><component name="KeymapManager"><active_keymap name="Mine" /></component></application>"#,
    );
    write(
        &dir.join("keymaps/Mine.xml"),
        r#"<keymap version="1" name="Mine" parent="Mac OS X 10.5+">
  <action id="GotoFile"><keyboard-shortcut first-keystroke="meta shift P" /></action>
</keymap>"#,
    );
    write(
        &o.join("editor-font.xml"),
        r#"<application><component name="DefaultFont">
  <option name="VERSION" value="1" />
  <option name="FONT_SIZE_2D" value="14.5" />
  <option name="FONT_FAMILY" value="Menlo" />
  <option name="USE_LIGATURES" value="true" />
</component></application>"#,
    );
    write(
        &o.join("editor.xml"),
        r#"<application><component name="EditorSettings"><option name="USE_SOFT_WRAPS" value="MAIN_EDITOR" /></component></application>"#,
    );
    write(
        &o.join("ide.general.xml"),
        r#"<application><component name="GeneralSettings">
  <option name="autoSaveIfInactive" value="true" />
  <option name="inactiveTimeout" value="30" />
</component></application>"#,
    );
    write(
        &o.join("filetypes.xml"),
        r#"<application><component name="FileTypeManager" version="19"><extensionMap>
  <mapping ext="jinja" type="HTML" />
</extensionMap></component></application>"#,
    );
    // The theme plugin: only its descriptor and theme files are read.
    let jar = dir.join("plugins/nightfall/lib/nightfall.jar");
    std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(File::create(&jar).unwrap());
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for (name, text) in [
        (
            "META-INF/plugin.xml".to_owned(),
            r#"<idea-plugin><extensions defaultExtensionNs="com.intellij">
  <themeProvider id="test-nightfall" path="/themes/Nightfall.theme.json" />
</extensions></idea-plugin>"#
                .to_owned(),
        ),
        (
            "themes/Nightfall.theme.json".to_owned(),
            themes_fixture("Nightfall.theme.json"),
        ),
        ("themes/Nightfall.xml".to_owned(), themes_fixture("Nightfall.xml")),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    // A folder unused for a year: under Other Options.
    let old = root.join("PyCharm2025.1/options/other.xml");
    write(&old, "<application/>");
    let year = SystemTime::now() - Duration::from_secs(365 * 24 * 60 * 60);
    File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(year)
        .unwrap();
}

fn open(cx: &mut TestAppContext, with_reference: bool) -> H<'_> {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let file = config.join("settings.toml");
    let root = dir.path().join("JetBrains");
    std::fs::create_dir_all(&root).unwrap();
    if with_reference {
        reference(&root);
    }
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let done = Rc::new(Cell::new(false));
    let finished = done.clone();
    let store_file = file.clone();
    cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store_file);
        theme_switch::load(cx, Some(config.join("themes")), None);
        workbench::keymap::active::load(cx, Some(config.join("keymaps")));
        first_run::open(
            Start {
                title: "Test IDE".into(),
                reference_root: Some(root),
                activate: false,
            },
            cx,
            move |_| finished.set(true),
        );
    });
    let mut h = H {
        cx,
        file,
        done,
        _dir: dir,
    };
    h.settle();
    h
}

#[gpui::test]
fn the_import_brings_the_reference_settings_and_preselects_them(cx: &mut TestAppContext) {
    let mut h = open(cx, true);
    let s = h.state();
    assert_eq!(s["page"], json!("import"), "{s}");
    assert_eq!(s["main"][0]["name"], json!("PyCharm2026.2"));
    assert_eq!(s["main"][0]["label"], json!("Reference IDE 2026.2"));
    assert_eq!(s["main"][0]["used"], json!("Used today"));
    assert_eq!(s["other"][0]["name"], json!("PyCharm2025.1"));
    assert_eq!(s["decided"], Value::Null);

    h.invoke("first-run.import", json!({}));
    let s = h.state();
    assert_eq!(s["page"], json!("theme"), "{s}");
    assert_eq!(s["kind"], json!("dark"));
    // The plugin's theme is the theme, first among the custom ones.
    let theme = h.cx.update(|cx| theme_switch::current_theme(cx));
    let name = h.cx.update(|cx| {
        theme_switch::catalog(cx)
            .get(&theme)
            .map(|e| e.name.clone())
            .unwrap()
    });
    assert_eq!(name, "Nightfall");
    assert_eq!(s["chosen"], json!(theme));
    assert!(s["cards"].as_array().unwrap().contains(&json!(theme)), "{s}");
    assert_eq!(h.get("appearance.editor-scheme"), Value::Null, "its own scheme");
    assert_eq!(h.get("editor.font.family"), json!("Menlo"));
    assert_eq!(h.get("editor.font.size"), json!(14.5));
    assert_eq!(h.get("editor.font.ligatures"), json!(true));
    assert_eq!(h.get("editor.soft-wraps.enabled"), json!(true));
    assert_eq!(h.get("system.files.save-on-idle"), json!(true));
    assert_eq!(h.get("system.files.idle-seconds"), json!(30));
    assert_eq!(
        h.get("file-types.associations"),
        json!([{"pattern": "*.jinja", "type": "HTML"}])
    );
    assert_eq!(h.get("import.source"), json!("PyCharm2026.2"));
    assert_eq!(h.get("keymap.name"), json!("Mine"));
    let report = h.cx.update(first_run::take_report).expect("a balloon");
    assert_eq!(report.title.as_ref(), "Settings imported");
    for part in [
        "From Reference IDE 2026.2",
        "the theme Nightfall",
        "the keymap Mine (based on macOS)",
        "the editor font (Menlo, 14.5, ligatures)",
        "soft wraps (on for *)",
        "autosave and file synchronization",
        "1 file type association",
    ] {
        assert!(report.content.contains(part), "{part}: {}", report.content);
    }

    // Choose Keymap: the imported keymap is chosen, with its shortcut.
    h.invoke("first-run.continue", json!(null));
    let s = h.state();
    assert_eq!(s["page"], json!("keymap"), "{s}");
    assert_eq!(s["chosen"], json!("Mine"));
    let cards = s["cards"].as_array().unwrap();
    let card = |name: &str| cards.iter().find(|c| c["name"] == json!(name)).unwrap();
    let go_to_file = |c: &Value| c["shortcuts"][2]["keys"].clone();
    assert_eq!(card("Mine")["shortcuts"][2]["name"], json!("Go to File"));
    let macos = cfg!(target_os = "macos");
    assert_eq!(
        go_to_file(card("Mine")),
        json!(if macos { "\u{21e7}\u{2318}P" } else { "Shift+Win+P" }),
        "{s}"
    );
    assert_ne!(go_to_file(card("Mine")), go_to_file(card(cards[0]["name"].as_str().unwrap())));
    // Another card switches at once; Back and forward again keep it.
    h.invoke("first-run.select-keymap", json!({"name": "VSCode"}));
    assert_eq!(h.get("keymap.name"), json!("VSCode"));
    h.invoke("first-run.back", json!(null));
    assert_eq!(h.state()["page"], json!("theme"));
    h.invoke("first-run.continue", json!(null));
    assert_eq!(h.state()["chosen"], json!("VSCode"));

    // Continue on the last page closes the window and goes on.
    h.invoke("first-run.continue", json!(null));
    assert!(h.window().is_none());
    assert!(h.done.get());
    let text = std::fs::read_to_string(&h.file).unwrap();
    assert!(
        text.contains("[import]\nsource = \"PyCharm2026.2\""),
        "{text}"
    );
    // The window comes once: the decision is stored.
    assert!(!h.cx.update(|cx| first_run::undecided(cx)));
}

#[gpui::test]
fn skip_import_keeps_the_defaults_and_records_the_choice(cx: &mut TestAppContext) {
    let mut h = open(cx, true);
    h.invoke("first-run.skip", json!(null));
    let s = h.state();
    assert_eq!(s["page"], json!("theme"), "{s}");
    assert_eq!(s["buttons"], json!(["first-run-continue", "first-run-back"]));
    assert_eq!(h.get("import.source"), json!("skipped"));
    assert_eq!(s["chosen"], json!("islands-dark"));
    assert_eq!(h.get("editor.font.size"), Value::Null);
    assert_eq!(h.cx.update(first_run::take_report).map(|n| n.title), None);
    // Light shows the light themes and switches to the first; Dark back.
    h.invoke("first-run.theme-kind", json!({"kind": "light"}));
    let s = h.state();
    assert_eq!(s["kind"], json!("light"));
    assert_eq!(s["chosen"], json!("islands-light"));
    assert_eq!(s["cards"], json!(["islands-light", "pyoxide-light"]));
    h.invoke("first-run.select-theme", json!({"id": "pyoxide-light"}));
    h.invoke("first-run.theme-kind", json!({"kind": "dark"}));
    assert_eq!(h.state()["chosen"], json!("islands-dark"));
    h.invoke("first-run.theme-kind", json!({"kind": "light"}));
    assert_eq!(h.state()["chosen"], json!("pyoxide-light"), "the last light one");
    // Back goes to the import page; closing the window goes on.
    h.invoke("first-run.back", json!(null));
    assert_eq!(h.state()["page"], json!("import"));
    h.invoke("first-run.cancel", json!(null));
    assert!(h.window().is_none());
    assert!(h.done.get());
    assert_eq!(h.get("import.source"), json!("skipped"));
}

#[gpui::test]
fn without_reference_settings_the_window_opens_on_choose_theme(cx: &mut TestAppContext) {
    let mut h = open(cx, false);
    let s = h.state();
    assert_eq!(s["page"], json!("theme"), "{s}");
    assert_eq!(s["buttons"], json!(["first-run-continue"]), "no Back");
    h.invoke("first-run.back", json!(null));
    assert_eq!(h.state()["page"], json!("theme"));
    h.invoke("first-run.continue", json!(null));
    h.invoke("first-run.continue", json!(null));
    assert!(h.done.get());
    assert_eq!(h.get("import.source"), json!("skipped"));
}

#[gpui::test]
fn other_options_list_older_folders_and_refuse_other_folders(cx: &mut TestAppContext) {
    let mut h = open(cx, true);
    h.invoke("first-run.other-options", json!(null));
    let popup = h.state()["popup"].clone();
    let labels: Vec<String> = popup["levels"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            i["label"]
                .as_str()
                .or(i["header"].as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| "-".to_owned())
        })
        .collect();
    assert_eq!(
        labels,
        [
            "Local Settings",
            "Reference IDE 2025.1",
            "-",
            "Config or Installation Directory\u{2026}"
        ],
        "{popup}"
    );
    // The popup takes the commands while it is open.
    h.invoke("menu.cancel", json!(null));
    assert!(h.state()["popup"].is_null());
    let folder = h._dir.path().join("not-a-config");
    std::fs::create_dir_all(&folder).unwrap();
    h.invoke(
        "first-run.choose-folder",
        json!({"path": folder.to_string_lossy()}),
    );
    let s = h.state();
    assert_eq!(s["page"], json!("import"));
    assert!(
        s["error"]
            .as_str()
            .unwrap()
            .ends_with("does not appear to be this IDE's settings or installation directory"),
        "{s}"
    );
    assert!(s["popup"].is_null());
    // An older folder imports too.
    h.invoke("first-run.import", json!({"folder": "PyCharm2025.1"}));
    assert_eq!(h.state()["page"], json!("theme"));
    assert_eq!(h.get("import.source"), json!("PyCharm2025.1"));
}
