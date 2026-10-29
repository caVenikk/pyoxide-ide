//! Settings > Tools > Terminal > Font Settings
//! (tests/scenarios/settings/terminal-font.md): the page opens with the font
//! the terminal has (the editor font until one is chosen), a field left
//! alone stays inherited, Apply reaches the terminal views at once, and the
//! font and its fallback are chosen from lists of monospaced families.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use workbench::{Product, Workbench};

struct H<'a> {
    cx: &'a mut TestAppContext,
    window: WindowHandle<Workbench>,
    /// `settings.toml`.
    file: PathBuf,
    _dir: tempfile::TempDir,
}

impl H<'_> {
    fn draw(&mut self) {
        self.cx.run_until_parked();
        for window in self.cx.update(|cx| cx.windows()) {
            let _ = self
                .cx
                .update_window(window, |_, window, cx| window.draw(cx).clear());
        }
    }

    fn invoke(&mut self, id: &str, args: Value) -> Value {
        let out = self
            .cx
            .update_window(self.window.into(), |_, window, cx| {
                ui::command::invoke(id, args, window, cx)
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        self.draw();
        out
    }

    fn edit(&mut self, field: &str, value: Value) {
        self.invoke("settings.edit", json!({"field": field, "value": value}));
    }

    fn page(&mut self) -> Value {
        self.invoke("settings.state", json!(null))["fields"].clone()
    }

    fn modified(&mut self) -> Value {
        self.invoke("settings.state", json!(null))["modified"].clone()
    }

    /// The font the terminal views draw with now.
    fn terminal(&mut self) -> terminal_ui::TerminalFont {
        self.cx
            .update(|cx| cx.try_global::<terminal_ui::TerminalFont>().cloned())
            .expect("a terminal font")
    }

    fn get(&mut self, key: &str) -> Value {
        self.cx
            .update(|cx| settings_ui::store::get_key(cx, key))
            .unwrap_or(Value::Null)
    }

    fn file_text(&mut self) -> String {
        self.cx.executor().advance_clock(Duration::from_secs(1));
        self.draw();
        std::fs::read_to_string(&self.file).unwrap_or_default()
    }
}

fn setup<'a>(cx: &'a mut TestAppContext, settings: &str) -> H<'a> {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config").join("settings.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, settings).unwrap();
    let product = Product {
        name: "Test IDE",
        ..Product::default()
    };
    let store = file.clone();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        settings_ui::store::load(cx, store);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let mut h = H {
        cx,
        window,
        file,
        _dir: dir,
    };
    h.draw();
    h
}

#[gpui::test]
fn the_page_edits_the_terminal_font_and_applies_it_at_once(cx: &mut TestAppContext) {
    let mut h = setup(cx, "[editor.font]\nfamily = \"Menlo\"\nsize = 15.0\n");
    h.invoke("settings.open", json!({"page": "terminal"}));
    // Tools > Terminal.
    let s = h.invoke("settings.state", json!(null));
    assert_eq!(s["breadcrumbs"], json!(["Tools", "Terminal"]), "{s}");

    // Until a font is chosen the terminal has the editor font; the
    // spacings are 1.
    let page = h.page();
    assert_eq!(page["family"], json!("Menlo"), "{page}");
    assert_eq!(page["fallback"], json!("Monospaced"), "{page}");
    assert_eq!(page["size"], json!("15.0"), "{page}");
    assert_eq!(page["line-height"], json!("1.0"), "{page}");
    assert_eq!(page["column-width"], json!("1.0"), "{page}");
    assert_eq!(h.modified(), json!([]));
    let font = h.terminal();
    assert_eq!(font.family.as_deref(), Some("Menlo"));
    assert_eq!(font.size, Some(15.0));

    // A size and spacings; the terminal waits for Apply.
    h.edit("size", json!("12.55"));
    h.edit("line-height", json!(1.2));
    h.edit("column-width", json!("1.05"));
    assert_eq!(h.modified(), json!(["terminal"]));
    assert_eq!(h.terminal().size, Some(15.0));
    h.invoke("settings.apply", json!(null));
    let font = h.terminal();
    assert_eq!(font.size, Some(12.6));
    assert_eq!(font.line_spacing, 1.2);
    assert_eq!(font.column_spacing, 1.05);
    // The family was left alone: it still follows the editor font.
    assert_eq!(h.get("terminal.font.family"), Value::Null);
    assert_eq!(h.get("terminal.font.size"), json!(12.6));
    let text = h.file_text();
    assert!(text.contains("[terminal.font]"), "{text}");
    assert!(text.contains("column-spacing = 1.05"), "{text}");
    assert!(!text.contains("[terminal.font]\nfamily"), "{text}");
    assert_eq!(h.modified(), json!([]));
    let page = h.page();
    assert_eq!(page["size"], json!("12.6"), "{page}");
    assert_eq!(page["line-height"], json!("1.2"), "{page}");
    assert_eq!(page["column-width"], json!("1.05"), "{page}");

    // The editor font changes: the terminal's family follows it, its
    // size was chosen and stays.
    h.cx.update(|cx| {
        settings_ui::store::set_key(cx, "editor.font.family", json!("Courier")).unwrap();
    });
    h.draw();
    let font = h.terminal();
    assert_eq!(font.family.as_deref(), Some("Courier"));
    assert_eq!(font.size, Some(12.6));

    // Numbers that are no numbers are the defaults; out of range they are
    // kept in range (there is no error).
    h.edit("size", json!("big"));
    h.edit("line-height", json!("9"));
    h.invoke("settings.apply", json!(null));
    let page = h.page();
    assert_eq!(page["size"], json!("13.0"), "{page}");
    assert_eq!(page["line-height"], json!("3.0"), "{page}");
    assert_eq!(h.terminal().line_spacing, 3.0);

    // Revert changes goes back to what is stored.
    h.edit("column-width", json!("2"));
    assert_eq!(h.modified(), json!(["terminal"]));
    h.invoke("settings.revert", json!(null));
    assert_eq!(h.page()["column-width"], json!("1.05"));
}

#[gpui::test]
fn the_font_and_the_fallback_are_chosen_from_lists(cx: &mut TestAppContext) {
    let mut h = setup(cx, "[terminal.font]\nfallback = \"Symbols Nerd Font\"\n");
    h.invoke("settings.open", json!({"page": "terminal"}));
    let page = h.page();
    assert_eq!(page["fallback"], json!("Symbols Nerd Font"), "{page}");
    assert_eq!(h.terminal().fallback.as_deref(), Some("Symbols Nerd Font"));

    // The lists open on request; the fallback's first row stands for the
    // platform's own fallback and the main font's list has none.
    h.edit("list", json!("fallback"));
    let page = h.page();
    assert_eq!(page["list"], json!("fallback"), "{page}");
    assert_eq!(page["rows"][0], json!({"label": "Monospaced"}), "{page}");
    h.edit("choose", json!("Monospaced"));
    let page = h.page();
    assert_eq!(page["list"], Value::Null, "{page}");
    assert_eq!(page["fallback"], json!("Monospaced"), "{page}");
    h.edit("list", json!("family"));
    let page = h.page();
    assert_eq!(page["list"], json!("family"), "{page}");
    assert!(
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("header").is_none() && r["label"] != json!("Monospaced")),
        "{page}"
    );
    h.edit("list", json!(null));

    h.edit("family", json!("JetBrainsMono Nerd Font"));
    h.invoke("settings.apply", json!(null));
    let font = h.terminal();
    assert_eq!(font.family.as_deref(), Some("JetBrainsMono Nerd Font"));
    // The platform's fallback: no font is added.
    assert_eq!(font.fallback, None);
    assert_eq!(
        h.get("terminal.font.family"),
        json!("JetBrainsMono Nerd Font")
    );
}
