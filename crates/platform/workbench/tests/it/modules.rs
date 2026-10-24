//! Modules (ADR 0022) with the fixture module crate
//! (`tests/fixture-module`), built as a module's crate is: what
//! `Product::with_module` merges and in which order, the module's key
//! bindings joined after the workbench's and checked with the exported
//! keymap harness, and documents routed to the language services of their
//! language.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use code_intel::fake::{Call, FakeCodeIntel};
use code_intel::{CodeIntel, DocumentId, ObjectTarget};
use gpui::{App, AppContext as _, Global, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use ui::icons::{IconName, IconSvg};
use workbench::file_types::FileTypes;
use workbench::keymap::{self, Binding, Keymap};
use workbench::test_support::keymap::{Harness, check_area, check_joined_once};
use workbench::{
    CodeIntelHook, CodeIntelProvider, FileAssociation, FileType, Init, Product, Workbench,
};
use workbench_fixture_module as fixture;

/// Services of every language, as a product's are.
struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }
}

/// A Python file type, an association of `data.fix` with it, an icon of
/// the module's name and services of every language.
fn product(python: Arc<FakeCodeIntel>) -> Product {
    Product {
        name: "Test IDE",
        file_types: vec![FileType {
            language_id: Some("python"),
            ..FileType::plain("Python", &["py"], IconName::FILE_TEXT)
        }],
        file_associations: vec![FileAssociation {
            mask: "data.fix",
            file_type: "Python",
        }],
        icons: vec![IconSvg {
            name: "fixture/file",
            light: b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"/>",
            dark: None,
        }],
        code_intel: vec![CodeIntelHook::new(Provider(python))],
        ..Product::default()
    }
}

/// The order the `init` functions ran in.
#[derive(Default)]
struct Ran(Vec<&'static str>);

impl Global for Ran {}

#[gpui::test]
fn with_module_appends_after_the_product_and_the_product_wins(cx: &mut TestAppContext) {
    let mut base = product(FakeCodeIntel::new());
    base.init = vec![Init::new(|cx| cx.default_global::<Ran>().0.push("product"))];
    let mut module = fixture::module(FakeCodeIntel::new());
    let module_init = module.init.take().unwrap();
    module.init = Some(Init::new(move |cx| {
        module_init.run(cx);
        cx.default_global::<Ran>().0.push("module");
    }));
    // The module claims the product's association too.
    module.file_associations = vec![
        FileAssociation {
            mask: "data.fix",
            file_type: "Fixture",
        },
        FileAssociation {
            mask: "notes.txt",
            file_type: "Fixture",
        },
    ];
    let merged = base.with_module(module);

    let names: Vec<&str> = merged.file_types.iter().map(|t| t.name).collect();
    assert_eq!(names, ["Python", "Fixture"]);
    let masks: Vec<&str> = merged.file_associations.iter().map(|a| a.mask).collect();
    assert_eq!(masks, ["data.fix", "data.fix", "notes.txt"]);
    // The product's icon of the same name stays.
    assert_eq!(merged.icons.len(), 1);
    assert!(merged.icons[0].light.ends_with(b"/>"));
    assert_eq!(merged.keymaps, [fixture::KEYMAPS]);
    let languages: Vec<&[&str]> = merged.code_intel.iter().map(|h| h.languages()).collect();
    assert_eq!(languages, [&[][..], &[fixture::LANGUAGE][..]]);
    assert_eq!(merged.init.len(), 2);

    // The product's association wins over the module's for one mask; the
    // module's own types and masks work.
    let types = FileTypes::with_associations(&merged.file_types, &merged.file_associations);
    assert_eq!(types.for_name("data.fix").unwrap().name, "Python");
    assert_eq!(types.for_name("notes.txt").unwrap().name, "Fixture");
    assert_eq!(types.for_name("q.fix").unwrap().name, "Fixture");
    assert_eq!(types.for_name("a.py").unwrap().name, "Python");
    // A product type that claims the module's extension wins too.
    let mut claiming = merged.file_types.clone();
    claiming[0] = FileType::plain("Python", &["py", "fix"], IconName::FILE_TEXT);
    let types = FileTypes::with_associations(&claiming, &[]);
    assert_eq!(types.for_name("q.fix").unwrap().name, "Python");

    // `init` functions run in order: the product's, then the module's,
    // which registers the module's command.
    let window = cx.update(|cx| {
        workbench::init(cx, &merged);
        workbench::open_main_window(cx, Rc::new(merged))
    });
    cx.run_until_parked();
    cx.update(|cx| assert_eq!(cx.global::<Ran>().0, ["product", "module"]));
    invoke(cx, window, "fixture.ping", json!(null));
    cx.update(|cx| assert_eq!(fixture::pings(cx), 1));
}

#[test]
fn the_module_keymap_folders_are_joined_once() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-module/keymaps");
    check_joined_once(&folder.join("macos"), fixture::KEYMAPS.macos).unwrap();
    check_joined_once(&folder.join("default"), fixture::KEYMAPS.default).unwrap();
    // A file joined twice fails.
    let twice = format!("{0}{0}", fixture::KEYMAPS.macos);
    assert!(check_joined_once(&folder.join("macos"), &twice).is_err());
}

/// The fixture module's one command: a ping counted.
fn check(h: &mut Harness, b: &Binding) -> bool {
    match b.command.as_str() {
        "fixture.ping" => {
            let before = h.cx.update(|cx| fixture::pings(cx));
            h.press(&b.keys);
            assert_eq!(h.cx.update(|cx| fixture::pings(cx)), before + 1);
            true
        }
        _ => false,
    }
}

fn with_fixture() -> Product {
    Product {
        name: "Test IDE",
        ..Product::default()
    }
    .with_module(fixture::module(FakeCodeIntel::new()))
}

#[gpui::test]
fn every_macos_binding_of_the_module_works(cx: &mut TestAppContext) {
    let area = fixture::KEYMAPS.macos;
    check_area(cx, with_fixture(), true, area, None, check);
}

#[gpui::test]
fn every_windows_and_linux_binding_of_the_module_works(cx: &mut TestAppContext) {
    let area = fixture::KEYMAPS.default;
    check_area(cx, with_fixture(), false, area, None, check);
}

#[gpui::test]
fn the_keymaps_in_use_have_the_module_bindings(cx: &mut TestAppContext) {
    let product = with_fixture();
    cx.update(|cx| workbench::init(cx, &product));
    for name in [
        keymap::MACOS_NAME,
        keymap::WINDOWS_NAME,
        keymap::VSCODE_NAME,
        keymap::VSCODE_MACOS_NAME,
    ] {
        let keymap = cx.update(|cx| keymap::active::resolve(name, cx)).unwrap();
        let macos = name == keymap::MACOS_NAME || name == keymap::VSCODE_MACOS_NAME;
        let keys = if macos {
            "cmd-alt-shift-f9"
        } else {
            "ctrl-alt-shift-f9"
        };
        let ping = keymap.bindings.iter().find(|b| b.command == "fixture.ping");
        assert_eq!(ping.map(|b| b.keys.as_str()), Some(keys), "{name}");
    }
    // Without the module the predefined keymaps are the workbench's.
    let plain = keymap::active::resolve_with(keymap::MACOS_NAME, &[]).unwrap();
    assert_eq!(plain, Keymap::parse(keymap::MACOS).unwrap());
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    cx.run_until_parked();
    out
}

fn opened(fake: &FakeCodeIntel) -> Vec<DocumentId> {
    fake.calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::DidOpen { doc, .. } => Some(doc),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn documents_go_to_the_services_of_their_language(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root: PathBuf = native_path::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a.py"), "a = 1\n").unwrap();
    std::fs::write(root.join("q.fix"), "ping\n").unwrap();
    let python = FakeCodeIntel::new();
    let module = FakeCodeIntel::new();
    let product = product(python.clone()).with_module(fixture::module(module.clone()));
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.executor().allow_parking();
    cx.run_until_parked();
    let path = |name: &str| root.join(name).display().to_string();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({ "path": root.display().to_string() }),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({ "path": path("a.py") }),
    );
    invoke(
        cx,
        window,
        "project.open-file",
        json!({ "path": path("q.fix") }),
    );

    // The module's services get only the documents of its language; the
    // product's serve every language.
    let (py, fix) = (
        DocumentId::new(root.join("a.py")),
        DocumentId::new(root.join("q.fix")),
    );
    assert_eq!(opened(&module), std::slice::from_ref(&fix));
    assert_eq!(opened(&python), [py, fix]);

    // The UI asks the provider of a document's language.
    let view = window
        .read_with(cx, |w, _| w.code_intel().cloned())
        .unwrap()
        .unwrap();
    cx.update(|cx| {
        let view = view.read(cx);
        assert_eq!(view.hooks().len(), 2);
        assert_eq!(
            view.hook_for(fixture::LANGUAGE).languages(),
            [fixture::LANGUAGE]
        );
        assert!(view.hook_for("python").languages().is_empty());
    });

    // Closing the project shuts both down.
    invoke(cx, window, "tabs.close", json!(null));
    assert!(
        module
            .calls()
            .iter()
            .any(|c| matches!(c, Call::DidClose { .. }))
    );
}

#[gpui::test]
fn object_targets_go_to_the_handler_of_their_scheme(cx: &mut TestAppContext) {
    let product = with_fixture();
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    let opened: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
    let sink = opened.clone();
    cx.update(|cx| {
        workbench::object_targets::register(
            cx,
            "db",
            Rc::new(move |target, _, _| sink.borrow_mut().push(target.id.clone())),
        )
    });
    let table = ObjectTarget::new("db", "main/orders", "orders");
    let other = ObjectTarget::new("web", "x", "x");
    let results = cx
        .update_window(window.into(), |_, window, cx| {
            (
                workbench::object_targets::open(&table, window, cx),
                workbench::object_targets::open(&other, window, cx),
            )
        })
        .unwrap();
    assert_eq!(results, (true, false));
    assert_eq!(*opened.borrow(), ["main/orders"]);
}
