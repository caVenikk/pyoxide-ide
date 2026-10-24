//! Test helpers for crates that build on the workbench, behind the
//! `test-support` feature: the keymap harness a module's crate checks its
//! key bindings with (ADR 0022), and that the workbench's own keymap checks
//! use.

pub mod keymap {
    //! Key bindings pressed as real keystrokes (`simulate_keystrokes`) in a
    //! headless main window: each keystroke must resolve to its command in
    //! its key context, and a check per command asserts its effect. A
    //! binding without a check fails, so a new binding comes with its check.
    //!
    //! A module's crate keeps its keymap folders next to its sources
    //! (`keymaps/macos/<area>.toml`, `keymaps/default/<area>.toml`), joins
    //! each folder into its [`KeymapFiles`](crate::KeymapFiles) and tests,
    //! per area and keymap, that every binding works with the complete
    //! keymap installed ([`check_area`]), and that each folder is joined once
    //! ([`check_joined_once`]).
    //!
    //! ```ignore
    //! #[gpui::test]
    //! fn every_macos_binding_works(cx: &mut gpui::TestAppContext) {
    //!     let product = Product::default().with_module(my_module());
    //!     let area = include_str!("../../keymaps/macos/db.toml");
    //!     keymap::check_area(cx, product, true, area, None, check);
    //! }
    //! ```

    use std::cell::RefCell;
    use std::ops::Deref;
    use std::path::Path;
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, App, AppContext as _, TestAppContext, WindowHandle};
    use serde_json::Value;

    pub use crate::keymap::check_joined_once;
    use crate::keymap::{self, Binding, Keymap};
    use crate::{Product, Workbench};

    /// The commands keystrokes resolved to, in order, since it was made
    /// (cleared by each press of a harness).
    #[derive(Clone, Default)]
    pub struct Dispatched(Rc<RefCell<Vec<String>>>);

    impl Dispatched {
        /// Records the commands of the keystrokes of the application from
        /// now on.
        pub fn observe(cx: &mut App) -> Self {
            let dispatched = Self::default();
            let sink = dispatched.0.clone();
            cx.observe_keystrokes(move |event, _, _| {
                if let Some(action) = &event.action {
                    sink.borrow_mut().push(action.name().to_owned());
                }
            })
            .detach();
            dispatched
        }
    }

    impl Deref for Dispatched {
        type Target = RefCell<Vec<String>>;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    /// What [`press_bindings`] needs of a harness.
    pub trait KeymapHost {
        fn dispatched(&self) -> &Dispatched;
    }

    /// Presses the bindings of an area file (`area_text`): `check` presses
    /// one and asserts its effect, and returns `false` for a command it has
    /// no check for; then the keystroke must have dispatched the binding's
    /// command (a double tap or a mouse button runs its command without a
    /// keystroke). Fails for bindings without a check.
    pub fn press_bindings<H: KeymapHost>(
        h: &mut H,
        area_text: &str,
        check: fn(&mut H, &Binding) -> bool,
    ) {
        let keymap = Keymap::parse(area_text).expect("the area file parses");
        let mut missing = Vec::new();
        for b in &keymap.bindings {
            eprintln!("checking `{}` -> {}", b.keys, b.command);
            if !check(h, b) {
                missing.push(format!("`{}` -> {}", b.keys, b.command));
                continue;
            }
            if keymap::parse_double_tap(&b.keys).is_some()
                || keymap::parse_mouse_button(&b.keys).is_some()
            {
                continue;
            }
            let dispatched = h.dispatched().borrow().clone();
            assert!(
                dispatched.contains(&b.command),
                "`{}` should dispatch {} in context {:?}, dispatched {dispatched:?}",
                b.keys,
                b.command,
                b.context
            );
        }
        assert!(missing.is_empty(), "bindings without a check: {missing:?}");
    }

    /// A headless main window of a product with the complete keymap
    /// installed: the workbench's default keymap (macOS, or Windows and
    /// Linux) with the bindings of the product's modules after it.
    pub struct Harness<'a> {
        pub cx: &'a mut TestAppContext,
        pub window: WindowHandle<Workbench>,
        pub dispatched: Dispatched,
    }

    impl KeymapHost for Harness<'_> {
        fn dispatched(&self) -> &Dispatched {
            &self.dispatched
        }
    }

    impl<'a> Harness<'a> {
        /// Starts the application with `product` and opens its main
        /// window, with the folder `root` open when given.
        pub fn new(
            cx: &'a mut TestAppContext,
            product: Product,
            macos: bool,
            root: Option<&Path>,
        ) -> Self {
            let keymap = Keymap::parse(&keymap::with_modules(macos, &product.keymaps))
                .expect("the keymap parses");
            let window = cx.update(|cx| {
                crate::init(cx, &product);
                cx.clear_key_bindings();
                let errors = keymap::bind(cx, &keymap);
                assert!(errors.is_empty(), "keymap errors: {errors:?}");
                crate::open_main_window(cx, Rc::new(product))
            });
            let dispatched = cx.update(Dispatched::observe);
            let mut h = Self {
                cx,
                window,
                dispatched,
            };
            h.draw();
            if let Some(root) = root {
                // Opening a project starts processes (git) and watchers.
                h.cx.executor().allow_parking();
                h.invoke(
                    "project.open-folder",
                    serde_json::json!({ "path": root.display().to_string() }),
                );
            }
            h
        }

        /// Runs what is due and draws every window.
        pub fn draw(&mut self) {
            self.cx.run_until_parked();
            for window in self.cx.update(|cx| cx.windows()) {
                let _ = self
                    .cx
                    .update_window(window, |_, window, cx| window.draw(cx).clear());
            }
        }

        /// The window that takes input: an open dialog, else the main one.
        pub fn target(&mut self) -> AnyWindowHandle {
            let main: AnyWindowHandle = self.window.into();
            self.cx
                .update(|cx| ui::dialog_window::modal_of(main, cx))
                .unwrap_or(main)
        }

        /// Runs a command of the registry in the main window and draws.
        pub fn invoke(&mut self, id: &str, args: Value) -> Value {
            let out = self
                .cx
                .update_window(self.window.into(), |_, window, cx| {
                    ui::command::invoke(id, args, window, cx)
                })
                .expect("the main window")
                .unwrap_or_else(|e| panic!("{id}: {e:?}"));
            self.draw();
            out
        }

        /// The workbench's state (`workbench.state`).
        pub fn state(&mut self) -> Value {
            self.invoke("workbench.state", Value::Null)
        }

        /// Types keystrokes (`"cmd-shift-a"`, space-separated for several)
        /// into the window that takes input, and draws.
        pub fn press(&mut self, keys: &str) {
            self.dispatched.borrow_mut().clear();
            let target = self.target();
            self.cx.simulate_keystrokes(target, keys);
            self.draw();
        }

        /// Reads the workbench.
        pub fn workbench<R>(&mut self, f: impl FnOnce(&Workbench, &App) -> R) -> R {
            self.window.read_with(self.cx, f).expect("the main window")
        }
    }

    /// Presses the bindings of one area file (`area_text`) of a module in a
    /// main window of `product` (which includes the module) with the
    /// complete keymap of macOS (`macos`) or of Windows and Linux installed.
    pub fn check_area(
        cx: &mut TestAppContext,
        product: Product,
        macos: bool,
        area_text: &str,
        root: Option<&Path>,
        check: fn(&mut Harness, &Binding) -> bool,
    ) {
        let mut h = Harness::new(cx, product, macos, root);
        press_bindings(&mut h, area_text, check);
    }
}
