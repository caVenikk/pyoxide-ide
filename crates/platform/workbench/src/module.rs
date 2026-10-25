//! Modules (ADR 0022): reusable sets of crates between the platform and the
//! products, the equivalent of a plugin of the reference that several of
//! its products share. A module describes what it contributes with a
//! [`Module`], and a product includes it with [`Product::with_module`].

use std::rc::Rc;

use code_intel_ui::CodeIntelHook;
use gpui::App;
use ui::icons::IconSvg;

use crate::app::{Product, StatusWidget};
use crate::file_types::{FileAssociation, FileType};
use crate::keymap::KeymapFiles;
use crate::tool_windows::ToolWindowHook;

/// A function that registers commands, tool windows, globals and settings
/// sections at start-up (the end of [`crate::init`]). It may own what a
/// module was given when it was made (a module's constructor takes what
/// only a product can supply).
#[derive(Clone)]
pub struct Init(Rc<dyn Fn(&mut App)>);

impl Init {
    pub fn new(f: impl Fn(&mut App) + 'static) -> Self {
        Self(Rc::new(f))
    }

    pub fn run(&self, cx: &mut App) {
        (self.0)(cx)
    }
}

impl From<fn(&mut App)> for Init {
    fn from(f: fn(&mut App)) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for Init {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Init(..)")
    }
}

/// What a module contributes to a product: the contribution fields of a
/// [`Product`], merged by [`Product::with_module`].
#[derive(Debug, Clone, Default)]
pub struct Module {
    /// The module's name, for messages (`database`).
    pub name: &'static str,
    /// File types of the module's languages; the product's own types win
    /// over them for a mask both claim.
    pub file_types: Vec<FileType>,
    /// The module's associations of file names with types; the product's
    /// win over them for the same mask.
    pub file_associations: Vec<FileAssociation>,
    /// Icons of the module (file types, tool windows); a product icon of
    /// the same name wins.
    pub icons: &'static [IconSvg],
    /// The module's pages of the Settings window.
    pub settings_pages: Vec<settings_ui::PageSpec>,
    /// The module's widgets in the status bar, after the product's.
    pub status_widgets: Vec<StatusWidget>,
    /// Registers the module's commands, tool windows and globals; runs
    /// after the product's.
    pub init: Option<Init>,
    /// The module's key bindings, joined after the workbench's.
    pub keymaps: Option<KeymapFiles>,
    /// Language services of the module's languages (a provider's
    /// `languages` say which documents it gets).
    pub code_intel: Vec<CodeIntelHook>,
    /// The module's tool windows, made for each main window.
    pub tool_windows: Vec<ToolWindowHook>,
}

impl Product {
    /// The product with a module's contributions: lists are appended
    /// after the product's own (file types, associations, icons, settings
    /// pages, status widgets, language services, key bindings), the
    /// product's file types, associations and icons still win over the
    /// module's, and `init` functions run in order.
    pub fn with_module(mut self, module: Module) -> Self {
        self.file_types.extend(module.file_types);
        self.file_associations.extend(module.file_associations);
        for icon in module.icons {
            if !self.icons.iter().any(|i| i.name == icon.name) {
                self.icons.push(*icon);
            }
        }
        self.settings_pages.extend(module.settings_pages);
        self.status_widgets.extend(module.status_widgets);
        self.init.extend(module.init);
        self.keymaps.extend(module.keymaps);
        self.code_intel.extend(module.code_intel);
        self.tool_windows.extend(module.tool_windows);
        self
    }
}
