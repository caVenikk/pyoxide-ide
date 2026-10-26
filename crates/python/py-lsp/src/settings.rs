//! Settings of the Python tools (ADR 0010).
//!
//! Shared, in `.oxide/project.toml`:
//!
//! ```toml
//! [python.tools.ruff]
//! enabled = true
//! lookup = "sdk"          # sdk | path | uvx
//! inspections = true
//! formatting = true
//! import-optimizer = true
//!
//! [python.type-engine]
//! name = "ty"             # ty | basedpyright
//! ```
//!
//! Every key is optional; a missing key, or one this build cannot read,
//! means Pyoxide's default. Defaults are not written. The `enabled` key
//! also says that a person (or the import of the reference's settings)
//! turned the tool on: such a tool may run through `uvx` without asking
//! (ADR 0010), so it is written even when it equals the default.
//!
//! - ruff runs while it is enabled (the default).
//! - The type server is the type engine's tool (`ty` by default). Its
//!   `enabled` defaults to true while it is the type engine; `enabled =
//!   false` turns the type server off. The other type tool does not run.
//! - `formatting` and `import-optimizer` are ruff's only.
//!
//! Personal, in `.oxide/workspace.toml`, by tool (`[python.tools.ruff]`):
//! `path`, a custom executable (absolute, or relative to the project root),
//! and `uvx` (`"allow"` or `"deny"`), the answer to the question whether a
//! default tool that is installed nowhere may run through `uvx`.

use std::path::{Path, PathBuf};

use project::{StoreError, store, workspace};
use serde::Serialize;

use crate::lookup::Lookup;

/// The table of a tool's shared settings: `python.tools.<id>`.
pub const TOOLS_TABLE: &str = "python.tools";
/// The table of the type engine choice.
pub const TYPE_ENGINE_TABLE: &str = "python.type-engine";

/// The oldest ty Pyoxide is tested with (the version ADR 0011 measured);
/// an older one still starts, with a warning (ADR 0011).
pub const MINIMUM_TY_VERSION: &str = "0.0.86";

const PYTHON_TABLE: &str = "python";
const ENABLED: &str = "enabled";
const LOOKUP: &str = "lookup";
const INSPECTIONS: &str = "inspections";
const FORMATTING: &str = "formatting";
const IMPORT_OPTIMIZER: &str = "import-optimizer";
const NAME: &str = "name";
const PATH: &str = "path";
const UVX: &str = "uvx";
const UVX_ALLOW: &str = "allow";
const UVX_DENY: &str = "deny";

/// A Python tool that runs as a language server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tool {
    Ruff,
    Ty,
    Basedpyright,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Ruff, Tool::Ty, Tool::Basedpyright];

    /// The id in settings and the server id (`ruff`); also the package name.
    pub fn id(self) -> &'static str {
        match self {
            Tool::Ruff => "ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "basedpyright",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Tool::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// The package that provides it (PyPI, for `uvx`).
    pub fn package(self) -> &'static str {
        self.id()
    }

    /// The executable that runs the server.
    pub fn executable(self) -> &'static str {
        match self {
            Tool::Ruff => "ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "basedpyright-langserver",
        }
    }

    /// The executable that answers `--version`: the server's own, except
    /// for basedpyright, whose language server has no `--version`; the
    /// command line tool installed next to it has.
    pub fn version_executable(self) -> &'static str {
        match self {
            Tool::Basedpyright => "basedpyright",
            _ => self.executable(),
        }
    }

    /// The arguments that start the server on its standard streams.
    ///
    /// The reference starts the pyright family as `<tool>-langserver
    /// server --stdio` (`PyrightLspClientDescriptor.lspArguments`); the
    /// language server ignores the extra word, and `--stdio` alone is what
    /// it documents and what was measured (ADR 0011).
    pub fn server_args(self) -> &'static [&'static str] {
        match self {
            Tool::Ruff | Tool::Ty => &["server"],
            Tool::Basedpyright => &["--stdio"],
        }
    }

    /// The name shown (the reference's tool names).
    pub fn name(self) -> &'static str {
        match self {
            Tool::Ruff => "Ruff",
            Tool::Ty => "ty",
            Tool::Basedpyright => "Basedpyright",
        }
    }

    /// It serves as a type server (ADR 0011).
    pub fn is_type_server(self) -> bool {
        self != Tool::Ruff
    }

    /// The oldest version Pyoxide is tested with, if it checks one.
    pub fn minimum_version(self) -> Option<&'static str> {
        match self {
            Tool::Ty => Some(MINIMUM_TY_VERSION),
            Tool::Ruff | Tool::Basedpyright => None,
        }
    }

    /// Its table in the settings files (`python.tools.ruff`).
    pub fn table(self) -> String {
        format!("{TOOLS_TABLE}.{}", self.id())
    }
}

/// The type server of a project (ADR 0011).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TypeEngine {
    #[default]
    Ty,
    Basedpyright,
}

impl TypeEngine {
    pub const ALL: [TypeEngine; 2] = [TypeEngine::Ty, TypeEngine::Basedpyright];

    pub fn id(self) -> &'static str {
        self.tool().id()
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "ty" => Some(TypeEngine::Ty),
            "basedpyright" => Some(TypeEngine::Basedpyright),
            _ => None,
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            TypeEngine::Ty => Tool::Ty,
            TypeEngine::Basedpyright => Tool::Basedpyright,
        }
    }

    /// The name shown (`Types: ty`).
    pub fn name(self) -> &'static str {
        self.tool().name()
    }
}

/// The shared settings of one tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolSettings {
    /// Runs as a language server. A type tool runs while it is the
    /// project's type engine.
    pub enabled: bool,
    pub lookup: Lookup,
    /// Its diagnostics and quick fixes.
    pub inspections: bool,
    /// Reformat Code goes to it (ruff).
    pub formatting: bool,
    /// Optimize Imports goes to it (ruff).
    pub import_optimizer: bool,
    /// The settings were written by a person or imported, not Pyoxide's
    /// defaults: lookup may end at `uvx` without asking (ADR 0010).
    pub explicit: bool,
}

impl ToolSettings {
    /// Pyoxide's defaults with ty as the type engine: ruff and ty are on
    /// (ADR 0011).
    pub fn default_for(tool: Tool) -> Self {
        Settings::default_tool(tool, TypeEngine::default())
    }

    /// Lookup may end at `uvx` without asking: the tool was turned on by a
    /// person or imported, or its lookup is `uvx` itself.
    pub fn uvx_without_asking(&self) -> bool {
        self.explicit || self.lookup == Lookup::Uvx
    }
}

/// The Python tools settings of a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub ruff: ToolSettings,
    pub ty: ToolSettings,
    pub basedpyright: ToolSettings,
    pub type_engine: TypeEngine,
}

impl Default for Settings {
    fn default() -> Self {
        Self::with_type_engine(TypeEngine::default())
    }
}

impl Settings {
    /// Pyoxide's defaults with `engine` as the type engine.
    pub fn with_type_engine(engine: TypeEngine) -> Self {
        Self {
            ruff: Self::default_tool(Tool::Ruff, engine),
            ty: Self::default_tool(Tool::Ty, engine),
            basedpyright: Self::default_tool(Tool::Basedpyright, engine),
            type_engine: engine,
        }
    }

    /// The defaults of a tool while `engine` is the type engine: ruff and
    /// the engine's tool are on.
    pub fn default_tool(tool: Tool, engine: TypeEngine) -> ToolSettings {
        let ruff = tool == Tool::Ruff;
        ToolSettings {
            enabled: ruff || tool == engine.tool(),
            lookup: Lookup::Sdk,
            inspections: true,
            formatting: ruff,
            import_optimizer: ruff,
            explicit: false,
        }
    }

    pub fn tool(&self, tool: Tool) -> &ToolSettings {
        match tool {
            Tool::Ruff => &self.ruff,
            Tool::Ty => &self.ty,
            Tool::Basedpyright => &self.basedpyright,
        }
    }

    pub fn tool_mut(&mut self, tool: Tool) -> &mut ToolSettings {
        match tool {
            Tool::Ruff => &mut self.ruff,
            Tool::Ty => &mut self.ty,
            Tool::Basedpyright => &mut self.basedpyright,
        }
    }

    /// The tools that run as servers: ruff when enabled, and the type
    /// engine's tool when enabled.
    pub fn servers(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if self.ruff.enabled {
            tools.push(Tool::Ruff);
        }
        let engine = self.type_engine.tool();
        if self.tool(engine).enabled {
            tools.push(engine);
        }
        tools
    }

    /// The type server that runs, if any.
    pub fn type_server(&self) -> Option<Tool> {
        let engine = self.type_engine.tool();
        self.tool(engine).enabled.then_some(engine)
    }

    /// Changes the type engine. The type tools nobody turned on explicitly
    /// follow it (the new engine's tool on, the other off).
    pub fn set_type_engine(&mut self, engine: TypeEngine) {
        self.type_engine = engine;
        for tool in [Tool::Ty, Tool::Basedpyright] {
            let settings = self.tool_mut(tool);
            if !settings.explicit {
                settings.enabled = tool == engine.tool();
            }
        }
    }

    /// The person chose the type engine (the type engine widget): its tool
    /// is on, as if enabled explicitly.
    pub fn select_type_engine(&mut self, engine: TypeEngine) {
        self.set_type_engine(engine);
        let settings = self.tool_mut(engine.tool());
        settings.enabled = true;
        settings.explicit = true;
    }
}

// ---- project.toml ----------------------------------------------------------

/// The project's settings: `.oxide/project.toml`, defaults for what is
/// not there. A file of a newer settings version is read as far as this
/// build understands it.
pub fn load(root: &Path) -> Result<Settings, StoreError> {
    let python = store::load_table(root, PYTHON_TABLE)?;
    Ok(from_python_table(python.as_ref()))
}

/// The tables of the stored settings.
struct Stored<'a> {
    python: Option<&'a toml::Table>,
}

impl<'a> Stored<'a> {
    fn table(&self, path: &[&str]) -> Option<&'a toml::Table> {
        path.iter()
            .try_fold(self.python?, |table, key| table.get(*key)?.as_table())
    }

    fn engine(&self) -> TypeEngine {
        self.table(&["type-engine"])
            .and_then(|t| t.get(NAME))
            .and_then(toml::Value::as_str)
            .and_then(TypeEngine::parse)
            .unwrap_or_default()
    }

    fn tool(&self, tool: Tool) -> Option<&'a toml::Table> {
        self.table(&["tools", tool.id()])
    }

    fn bool(&self, tool: Tool, key: &str) -> Option<bool> {
        self.tool(tool)?.get(key)?.as_bool()
    }

    fn lookup(&self, tool: Tool) -> Option<Lookup> {
        Lookup::parse(self.tool(tool)?.get(LOOKUP)?.as_str()?)
    }

    /// The settings of a tool as stored, defaults (for `engine`) for the
    /// rest.
    fn tool_settings(&self, tool: Tool, engine: TypeEngine) -> ToolSettings {
        let mut out = Settings::default_tool(tool, engine);
        if let Some(enabled) = self.bool(tool, ENABLED) {
            out.enabled = enabled;
            out.explicit = true;
        }
        if let Some(lookup) = self.lookup(tool) {
            out.lookup = lookup;
        }
        if let Some(inspections) = self.bool(tool, INSPECTIONS) {
            out.inspections = inspections;
        }
        if tool == Tool::Ruff {
            if let Some(formatting) = self.bool(tool, FORMATTING) {
                out.formatting = formatting;
            }
            if let Some(optimizer) = self.bool(tool, IMPORT_OPTIMIZER) {
                out.import_optimizer = optimizer;
            }
        }
        out
    }
}

fn from_python_table(python: Option<&toml::Table>) -> Settings {
    let stored = Stored { python };
    let engine = stored.engine();
    Settings {
        ruff: stored.tool_settings(Tool::Ruff, engine),
        ty: stored.tool_settings(Tool::Ty, engine),
        basedpyright: stored.tool_settings(Tool::Basedpyright, engine),
        type_engine: engine,
    }
}

/// Writes the project's settings: only the keys whose meaning differs
/// from what is stored, so unknown keys and values this build cannot read
/// stay as they are unless their setting changed. Defaults are removed
/// rather than written (except `enabled` of a tool turned on explicitly).
/// A settings file of a newer version is not rewritten
/// ([`StoreError::Newer`]). Returns whether the file was written.
pub fn save(root: &Path, settings: &Settings) -> Result<bool, StoreError> {
    let python = store::load_table(root, PYTHON_TABLE)?;
    let stored = Stored {
        python: python.as_ref(),
    };
    let engine = settings.type_engine;
    let mut written = false;

    if stored.engine() != engine {
        let name = (engine != TypeEngine::default()).then(|| toml::Value::from(engine.id()));
        written |= store::save_values(root, TYPE_ENGINE_TABLE, &[(NAME, name)])?;
    }

    for tool in Tool::ALL {
        let want = settings.tool(tool);
        // What the stored keys mean under the new type engine.
        let have = stored.tool_settings(tool, engine);
        let default = Settings::default_tool(tool, engine);
        let mut values: Vec<(&str, Option<toml::Value>)> = Vec::new();
        if (want.enabled, want.explicit) != (have.enabled, have.explicit) {
            let keep = want.explicit || want.enabled != default.enabled;
            values.push((ENABLED, keep.then_some(toml::Value::from(want.enabled))));
        }
        if want.lookup != have.lookup {
            let value =
                (want.lookup != default.lookup).then(|| toml::Value::from(want.lookup.id()));
            values.push((LOOKUP, value));
        }
        let mut flag = |key: &'static str, want: bool, have: bool, default: bool| {
            if want != have {
                values.push((key, (want != default).then_some(toml::Value::from(want))));
            }
        };
        flag(
            INSPECTIONS,
            want.inspections,
            have.inspections,
            default.inspections,
        );
        if tool == Tool::Ruff {
            flag(
                FORMATTING,
                want.formatting,
                have.formatting,
                default.formatting,
            );
            flag(
                IMPORT_OPTIMIZER,
                want.import_optimizer,
                have.import_optimizer,
                default.import_optimizer,
            );
        }
        if !values.is_empty() {
            written |= store::save_values(root, &tool.table(), &values)?;
        }
    }
    Ok(written)
}

/// The project has tool settings of its own (`[python.tools]` in
/// `project.toml`): the reference's are no longer imported.
pub fn has_tool_settings(root: &Path) -> Result<bool, StoreError> {
    Ok(store::load_table(root, TOOLS_TABLE)?.is_some())
}

/// Creates an empty `[python.tools]` table when there is none (it ends
/// the one-time import of the reference's settings). Returns whether the
/// file was written.
pub fn ensure_tools_table(root: &Path) -> Result<bool, StoreError> {
    if has_tool_settings(root)? {
        return Ok(false);
    }
    // The store creates a table only with a value in it: write a
    // placeholder and remove it, which leaves the table.
    const PLACEHOLDER: &str = "imported";
    store::save_values(root, TOOLS_TABLE, &[(PLACEHOLDER, Some(true.into()))])?;
    store::save_values(root, TOOLS_TABLE, &[(PLACEHOLDER, None)])?;
    Ok(true)
}

/// The person chose a type engine: it is stored with its tool turned on
/// explicitly. Returns the settings now in effect.
pub fn select_type_engine(root: &Path, engine: TypeEngine) -> Result<Settings, StoreError> {
    let mut settings = load(root)?;
    settings.select_type_engine(engine);
    save(root, &settings)?;
    Ok(settings)
}

// ---- workspace.toml --------------------------------------------------------

/// The person's custom executable of a tool (`.oxide/workspace.toml`),
/// relative paths resolved against the project root.
pub fn custom_path(root: &Path, tool: Tool) -> Result<Option<PathBuf>, StoreError> {
    let value = workspace::load_value(root, &tool.table(), PATH)?;
    Ok(value.filter(|v| !v.trim().is_empty()).map(|v| {
        let path = PathBuf::from(v);
        if path.is_relative() {
            root.join(path)
        } else {
            path
        }
    }))
}

/// Sets or removes the person's custom executable of a tool. Returns
/// whether the file was written (not when it holds that path already).
pub fn set_custom_path(root: &Path, tool: Tool, path: Option<&Path>) -> Result<bool, StoreError> {
    let table = tool.table();
    let value = path.map(|p| p.to_string_lossy().into_owned());
    if workspace::load_value(root, &table, PATH)? == value {
        return Ok(false);
    }
    workspace::save_value(root, &table, PATH, value.as_deref())
}

/// The person's answer whether a default tool installed nowhere may run
/// through `uvx` (ADR 0010); `None` while not asked.
pub fn uvx_consent(root: &Path, tool: Tool) -> Result<Option<bool>, StoreError> {
    let value = workspace::load_value(root, &tool.table(), UVX)?;
    Ok(match value.as_deref() {
        Some(UVX_ALLOW) => Some(true),
        Some(UVX_DENY) => Some(false),
        _ => None,
    })
}

/// Remembers (or forgets, with `None`) the answer of [`uvx_consent`].
pub fn set_uvx_consent(root: &Path, tool: Tool, allow: Option<bool>) -> Result<bool, StoreError> {
    let table = tool.table();
    let value = allow.map(|allow| if allow { UVX_ALLOW } else { UVX_DENY });
    if workspace::load_value(root, &table, UVX)?.as_deref() == value {
        return Ok(false);
    }
    workspace::save_value(root, &table, UVX, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(root: &Path) -> String {
        std::fs::read_to_string(store::settings_file(root)).unwrap()
    }

    #[test]
    fn tools_and_defaults() {
        assert_eq!(Tool::Ruff.table(), "python.tools.ruff");
        assert_eq!(Tool::parse("basedpyright"), Some(Tool::Basedpyright));
        assert_eq!(Tool::parse("pyright"), None);
        assert_eq!(Tool::Basedpyright.executable(), "basedpyright-langserver");
        assert_eq!(Tool::Basedpyright.version_executable(), "basedpyright");
        assert_eq!(Tool::Ty.version_executable(), "ty");
        assert_eq!(Tool::Basedpyright.server_args(), ["--stdio"]);
        assert_eq!(Tool::Ty.minimum_version(), Some(MINIMUM_TY_VERSION));
        assert!(!Tool::Ruff.is_type_server());
        assert_eq!(
            TypeEngine::parse("basedpyright"),
            Some(TypeEngine::Basedpyright)
        );
        assert_eq!(TypeEngine::parse("pyrefly"), None);
        assert_eq!(TypeEngine::Basedpyright.name(), "Basedpyright");
        let settings = Settings::default();
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Ty]);
        assert_eq!(settings.type_server(), Some(Tool::Ty));
        assert!(settings.tool(Tool::Ruff).formatting);
        assert!(!settings.tool(Tool::Ty).formatting);
        assert!(!settings.ruff.explicit);
        assert!(!settings.ruff.uvx_without_asking());
        assert!(!settings.basedpyright.enabled);
        assert_eq!(ToolSettings::default_for(Tool::Ty), settings.ty);
    }

    #[test]
    fn type_engine_choice() {
        let mut settings = Settings::default();
        settings.set_type_engine(TypeEngine::Basedpyright);
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Basedpyright]);
        assert!(!settings.ty.enabled && !settings.basedpyright.explicit);
        settings.select_type_engine(TypeEngine::Ty);
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Ty]);
        assert!(settings.ty.explicit && settings.ty.uvx_without_asking());
        // A type server turned off explicitly: ruff only.
        settings.ty.enabled = false;
        assert_eq!(settings.servers(), [Tool::Ruff]);
        assert_eq!(settings.type_server(), None);
    }

    #[test]
    fn nothing_is_written_for_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(load(root).unwrap(), Settings::default());
        assert!(!save(root, &Settings::default()).unwrap());
        assert!(!store::settings_file(root).exists());
        assert!(!has_tool_settings(root).unwrap());
    }

    #[test]
    fn settings_round_trip() {
        brand::install();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut settings = Settings::default();
        settings.ruff.enabled = true;
        settings.ruff.explicit = true;
        settings.ruff.lookup = Lookup::Path;
        settings.ruff.formatting = false;
        settings.basedpyright.lookup = Lookup::Uvx;
        settings.set_type_engine(TypeEngine::Basedpyright);
        assert!(save(root, &settings).unwrap());
        assert_eq!(load(root).unwrap(), settings);
        assert!(has_tool_settings(root).unwrap());
        let text = read(root);
        assert!(text.starts_with("# Pyoxide project settings."), "{text}");
        assert!(text.contains("version = 1"), "{text}");
        assert!(
            text.contains(
                "[python.tools.ruff]\nenabled = true\nlookup = \"path\"\nformatting = false\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("[python.type-engine]\nname = \"basedpyright\"\n"),
            "{text}"
        );
        // basedpyright is on as the engine's tool, not explicitly.
        assert!(
            !text.contains("[python.tools.basedpyright]\nenabled"),
            "{text}"
        );
        assert!(text.contains("lookup = \"uvx\""), "{text}");
        assert!(!text.contains("[python.tools.ty]"), "{text}");

        // Saving the same settings again writes nothing.
        assert!(!save(root, &settings).unwrap());

        // Back to the defaults: the keys go, the tables stay.
        let mut back = settings.clone();
        back.ruff = ToolSettings::default_for(Tool::Ruff);
        back.basedpyright.lookup = Lookup::Sdk;
        back.set_type_engine(TypeEngine::Ty);
        assert!(save(root, &back).unwrap());
        assert_eq!(load(root).unwrap(), Settings::default());
        let text = read(root);
        assert!(!text.contains("lookup"), "{text}");
        assert!(!text.contains("name ="), "{text}");
    }

    #[test]
    fn written_by_hand() {
        brand::install();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".oxide")).unwrap();
        let text = "version = 1\n\n\
                    [python.tools.ruff]\n\
                    # the CI pins its own ruff\n\
                    lookup = \"bogus\"\n\
                    future-key = 3\n\
                    inspections = false\n\n\
                    [python.tools.ty]\n\
                    enabled = false\n\n\
                    [python.type-engine]\n\
                    name = \"pyrefly\"\n";
        std::fs::write(store::settings_file(root), text).unwrap();
        let settings = load(root).unwrap();
        // Unknown values read as defaults.
        assert_eq!(settings.ruff.lookup, Lookup::Sdk);
        assert!(!settings.ruff.inspections);
        assert!(!settings.ruff.explicit);
        assert_eq!(settings.type_engine, TypeEngine::Ty);
        assert!(!settings.ty.enabled && settings.ty.explicit);
        assert_eq!(settings.servers(), [Tool::Ruff]);

        // Changing one setting keeps the rest of the file as it was.
        let mut changed = settings.clone();
        changed.ruff.inspections = true;
        assert!(save(root, &changed).unwrap());
        let after = read(root);
        assert!(after.contains("# the CI pins its own ruff"), "{after}");
        assert!(after.contains("lookup = \"bogus\""), "{after}");
        assert!(after.contains("future-key = 3"), "{after}");
        assert!(after.contains("name = \"pyrefly\""), "{after}");
        assert!(!after.contains("inspections"), "{after}");
        assert_eq!(load(root).unwrap(), changed);
    }

    #[test]
    fn a_newer_settings_file_is_not_rewritten() {
        brand::install();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".oxide")).unwrap();
        let text = "version = 2\n\n[python.tools.ruff]\nlookup = \"path\"\n";
        std::fs::write(store::settings_file(root), text).unwrap();
        // Read as far as understood.
        let settings = load(root).unwrap();
        assert_eq!(settings.ruff.lookup, Lookup::Path);
        let mut changed = settings;
        changed.ruff.lookup = Lookup::Uvx;
        assert!(matches!(
            save(root, &changed),
            Err(StoreError::Newer { version: 2, .. })
        ));
        assert_eq!(read(root), text);
        assert!(matches!(
            select_type_engine(root, TypeEngine::Basedpyright),
            Err(StoreError::Newer { .. })
        ));
    }

    #[test]
    fn choosing_the_type_engine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let settings = select_type_engine(root, TypeEngine::Basedpyright).unwrap();
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Basedpyright]);
        assert!(settings.basedpyright.uvx_without_asking());
        assert_eq!(load(root).unwrap(), settings);
        let text = read(root);
        assert!(
            text.contains("[python.tools.basedpyright]\nenabled = true\n"),
            "{text}"
        );
        assert!(
            text.contains("[python.type-engine]\nname = \"basedpyright\"\n"),
            "{text}"
        );
        // Back to ty: basedpyright stays explicitly on but does not run.
        let settings = select_type_engine(root, TypeEngine::Ty).unwrap();
        assert_eq!(settings.servers(), [Tool::Ruff, Tool::Ty]);
        assert_eq!(load(root).unwrap(), settings);
        assert!(!read(root).contains("name ="));
    }

    #[test]
    fn personal_paths_and_uvx_answers() {
        brand::install();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(custom_path(root, Tool::Ruff).unwrap(), None);
        assert!(!set_custom_path(root, Tool::Ruff, None).unwrap());
        assert!(!workspace::workspace_file(root).exists());

        let ruff = root.join("tools").join("ruff");
        assert!(set_custom_path(root, Tool::Ruff, Some(&ruff)).unwrap());
        assert!(!set_custom_path(root, Tool::Ruff, Some(&ruff)).unwrap());
        assert_eq!(custom_path(root, Tool::Ruff).unwrap(), Some(ruff));
        assert_eq!(custom_path(root, Tool::Ty).unwrap(), None);
        // Personal state only: the shared settings are not created.
        assert!(!store::settings_file(root).exists());
        assert!(root.join(".oxide").join(".gitignore").is_file());

        workspace::save_value(root, "python.tools.ty", "path", Some("bin/ty")).unwrap();
        assert_eq!(
            custom_path(root, Tool::Ty).unwrap(),
            Some(root.join("bin").join("ty"))
        );

        assert_eq!(uvx_consent(root, Tool::Ty).unwrap(), None);
        assert!(set_uvx_consent(root, Tool::Ty, Some(false)).unwrap());
        assert_eq!(uvx_consent(root, Tool::Ty).unwrap(), Some(false));
        assert!(set_uvx_consent(root, Tool::Ty, Some(true)).unwrap());
        assert!(!set_uvx_consent(root, Tool::Ty, Some(true)).unwrap());
        assert_eq!(uvx_consent(root, Tool::Ty).unwrap(), Some(true));
        let text = std::fs::read_to_string(workspace::workspace_file(root)).unwrap();
        assert!(text.contains("[python.tools.ty]"), "{text}");
        assert!(text.contains("uvx = \"allow\""), "{text}");

        assert!(set_custom_path(root, Tool::Ruff, None).unwrap());
        assert_eq!(custom_path(root, Tool::Ruff).unwrap(), None);
    }
}
