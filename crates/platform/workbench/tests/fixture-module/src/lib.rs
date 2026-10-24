//! A module for the workbench's tests (ADR 0022), built as a module's crate
//! is: it depends on the workbench, keeps its keymap folders next to its
//! sources and contributes through a `workbench::Module`. It has a file
//! type (`*.fix`, language id `fixture`), a language service provider for
//! that language, an icon, a command (`fixture.ping`, which counts) and its
//! key bindings.

use std::path::Path;
use std::sync::Arc;

use code_intel::CodeIntel;
use code_intel::fake::FakeCodeIntel;
use gpui::{App, Global};
use ui::icons::{IconName, IconSvg};
use workbench::{CodeIntelHook, CodeIntelProvider, FileType, Init, KeymapFiles, Module};

ui::ui_commands! {
    COMMANDS, scope = "Fixture";
    /// Count a ping.
    Ping => "fixture.ping", "Ping",
        "Counts a ping (the fixture module's command).";
}

/// The language id of the module's file type.
pub const LANGUAGE: &str = "fixture";

/// The module's key bindings: its keymap folders, joined.
pub const KEYMAPS: KeymapFiles = KeymapFiles {
    macos: concat!(include_str!("../keymaps/macos/fixture.toml")),
    default: concat!(include_str!("../keymaps/default/fixture.toml")),
};

/// The module's icon (of its file type).
pub const ICONS: &[IconSvg] = &[IconSvg {
    name: "fixture/file",
    light: br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect x="3" y="2" width="10" height="12" fill="#888"/></svg>"##,
    dark: None,
}];

/// How many pings the command counted.
#[derive(Default)]
pub struct Pings(pub usize);

impl Global for Pings {}

/// The pings so far.
pub fn pings(cx: &App) -> usize {
    cx.try_global::<Pings>().map_or(0, |p| p.0)
}

/// The module, with `intel` as the services of its language.
pub fn module(intel: Arc<FakeCodeIntel>) -> Module {
    Module {
        name: "fixture",
        file_types: vec![FileType {
            language_id: Some(LANGUAGE),
            ..FileType::plain("Fixture", &["fix"], IconName("fixture/file"))
        }],
        icons: ICONS,
        init: Some(Init::new(|cx| {
            ui::command::register(cx, COMMANDS);
            cx.on_action(|_: &Ping, cx| cx.default_global::<Pings>().0 += 1);
        })),
        keymaps: Some(KEYMAPS),
        code_intel: vec![CodeIntelHook::new(Provider(intel))],
        ..Module::default()
    }
}

/// The services of the module's language.
struct Provider(Arc<FakeCodeIntel>);

impl CodeIntelProvider for Provider {
    fn languages(&self) -> &[&'static str] {
        &[LANGUAGE]
    }

    fn start(&self, _: &Path, _: &mut App) -> Arc<dyn CodeIntel> {
        self.0.clone()
    }
}
