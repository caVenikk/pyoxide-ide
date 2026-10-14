//! The one-time import of the reference's Python tool settings (ADR 0010),
//! read while `project.toml` has no `[python.tools]` table. `.idea` is
//! never written.
//!
//! `.idea/pyLspTools.xml` holds, in the reference's current form, the
//! component `PyToolsState`: a map from package name to the tool's entry
//! (entries at their defaults are left out):
//!
//! ```xml
//! <component name="PyToolsState">
//!   <option name="tools">
//!     <map>
//!       <entry key="ruff">
//!         <value>
//!           <ToolEntry>
//!             <option name="enabled" value="true" />
//!             <option name="discoveryMode" value="PATH" />
//!             <option name="customPathToExecutable" value="/opt/tools/ruff" />
//!           </ToolEntry>
//!         </value>
//!       </entry>
//!     </map>
//!   </option>
//! </component>
//! ```
//!
//! and one component per tool with its features (`RuffConfiguration`:
//! `inspections`, `formatting`, `sortImports`; `TyConfiguration`,
//! `BasedpyrightConfiguration`: `inspections`). Older versions kept
//! `enabled`, `executableDiscoveryMode` and `pathToExecutable` in the
//! per-tool components; the reference migrates them once when the file
//! has no `PyToolsState`, and so does this import. Discovery modes:
//! `INTERPRETER` (Sdk), `PATH`, `UVX`.
//!
//! `.idea/pyTypeEngine.xml` holds `PyTypeEngineProjectSettings` with the
//! option `typeEngine` (`PYCHARM`, `PYREFLY`, `TY`).
//!
//! How the reference's settings become Pyoxide's:
//!
//! - A tool enabled in the reference is enabled explicitly (it may run
//!   through `uvx` without asking). A tool off in the reference (its
//!   default) keeps Pyoxide's default: Pyoxide has no engine of its own.
//! - Lookup modes and features are taken as they are; custom executables
//!   become personal paths in `.oxide/workspace.toml`.
//! - Pyright enabled (without basedpyright) becomes basedpyright, its fork
//!   with the closest semantics, with Pyright's lookup mode.
//! - The type engine: ty when the reference's type engine is ty or
//!   Pyrefly (not supported yet), or ty is enabled; basedpyright when it
//!   (or Pyright) is enabled; ty otherwise.
//! - What Pyoxide does differently is listed in [`Imported::notes`].
//! - Once imported, `[python.tools]` exists (empty if nothing else went
//!   there), so the import does not repeat.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use project::idea::IDEA_DIR;
use project::{StoreError, store};
use quick_xml::events::{BytesStart, Event};

use crate::lookup::Lookup;
use crate::settings::{self, Settings, Tool, TypeEngine};

/// The reference's file of the Python tools in `.idea`.
pub const TOOLS_FILE: &str = "pyLspTools.xml";
/// The reference's file of the type engine in `.idea`.
pub const TYPE_ENGINE_FILE: &str = "pyTypeEngine.xml";

const TOOLS_STATE: &str = "PyToolsState";
const TYPE_ENGINE_SETTINGS: &str = "PyTypeEngineProjectSettings";
const PYRIGHT: &str = "pyright";
const PYREFLY: &str = "pyrefly";

/// The note of an import that found Pyright enabled.
pub const NOTE_PYRIGHT: &str =
    "Pyright is enabled in the reference IDE: basedpyright, a fork of it, runs instead";
/// The note of an import that found Pyrefly enabled or as the type engine.
pub const NOTE_PYREFLY: &str = "Pyrefly is not supported yet: ty runs as the type server instead";

/// The reference's tool settings of a project, as Pyoxide's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The shared settings, Pyoxide's defaults where the reference sets
    /// nothing.
    pub settings: Settings,
    /// Custom executables, personal in Pyoxide.
    pub paths: BTreeMap<Tool, PathBuf>,
    /// What Pyoxide does differently from the reference's settings, for
    /// the log.
    pub notes: Vec<String>,
}

/// The reference's tool settings of a project; `None` when `.idea` has
/// none for ruff, ty, basedpyright, Pyright or Pyrefly (or the files
/// cannot be read).
pub fn read_tool_settings(root: &Path) -> Option<Imported> {
    let idea = root.join(IDEA_DIR);
    if !idea.is_dir() {
        return None;
    }
    let text = |file: &str| std::fs::read_to_string(idea.join(file)).ok();
    let tools = text(TOOLS_FILE)
        .and_then(|t| parse_tools(&t).ok())
        .unwrap_or_default();
    let engine = text(TYPE_ENGINE_FILE).and_then(|t| parse_type_engine(&t).ok().flatten());
    convert(&tools, engine.as_deref(), root)
}

/// Imports the reference's tool settings into `.oxide/project.toml` (and
/// custom executables into `.oxide/workspace.toml`), once: only while the
/// project has no tool settings of its own. Returns them when they were
/// written.
///
/// Run it after the import of the reference's exclusions
/// (`project::store::import_idea`): that one happens only while
/// `project.toml` does not exist, and this one may create it.
pub fn import_tool_settings(root: &Path) -> Result<Option<Imported>, StoreError> {
    if settings::has_tool_settings(root)? {
        return Ok(None);
    }
    let Some(mut imported) = read_tool_settings(root) else {
        return Ok(None);
    };
    // A type engine chosen in Pyoxide already stays.
    if store::load_table(root, settings::TYPE_ENGINE_TABLE)?.is_some() {
        let engine = settings::load(root)?.type_engine;
        imported.settings.set_type_engine(engine);
    }
    settings::save(root, &imported.settings)?;
    // Nothing shared to write (only custom paths, or notes): the table
    // still closes the one-time import.
    settings::ensure_tools_table(root)?;
    for (tool, path) in &imported.paths {
        settings::set_custom_path(root, *tool, Some(path))?;
    }
    Ok(Some(imported))
}

/// One tool in the reference: `PyToolsState`'s entry, or the older fields
/// of its component.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Entry {
    enabled: Option<bool>,
    lookup: Option<Lookup>,
    path: Option<String>,
}

/// What `pyLspTools.xml` holds.
#[derive(Debug, Default)]
struct ToolsFile {
    /// `PyToolsState`'s entries by package name; `None` without the
    /// component.
    state: Option<HashMap<String, Entry>>,
    /// The options of the other components, by component name.
    components: HashMap<String, HashMap<String, String>>,
}

impl ToolsFile {
    fn option(&self, tool: Tool, name: &str) -> Option<&str> {
        self.components
            .get(component_of(tool))?
            .get(name)
            .map(String::as_str)
    }

    /// The tools' entries: `PyToolsState`'s, or, in a file of an older
    /// version without it, the fields of the per-tool components.
    fn entries(&self) -> HashMap<String, Entry> {
        if let Some(state) = &self.state {
            return state.clone();
        }
        let mut out = HashMap::new();
        for (component, options) in &self.components {
            let Some(package) = package_of(component) else {
                continue;
            };
            let entry = Entry {
                enabled: options.get("enabled").and_then(|v| parse_bool(v)),
                lookup: options
                    .get("executableDiscoveryMode")
                    .and_then(|v| parse_mode(v)),
                path: options.get("pathToExecutable").cloned(),
            };
            out.insert(package.to_owned(), entry);
        }
        out
    }
}

/// The per-tool component of the reference.
fn component_of(tool: Tool) -> &'static str {
    match tool {
        Tool::Ruff => "RuffConfiguration",
        Tool::Ty => "TyConfiguration",
        Tool::Basedpyright => "BasedpyrightConfiguration",
    }
}

/// The package of a per-tool component of the reference.
fn package_of(component: &str) -> Option<&'static str> {
    Some(match component {
        "RuffConfiguration" => "ruff",
        "TyConfiguration" => "ty",
        "BasedpyrightConfiguration" => "basedpyright",
        "PyrightConfiguration" => "pyright",
        "PyreflyConfiguration" | "PyreflyConfiguration2" => "pyrefly",
        _ => return None,
    })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// A discovery mode of the reference (`ExecutableDiscoveryMode`).
fn parse_mode(value: &str) -> Option<Lookup> {
    match value.trim().to_ascii_uppercase().as_str() {
        "INTERPRETER" | "SDK" => Some(Lookup::Sdk),
        "PATH" => Some(Lookup::Path),
        "UVX" => Some(Lookup::Uvx),
        _ => None,
    }
}

/// `$PROJECT_DIR$` and `$USER_HOME$` in a path of the reference.
fn expand(path: &str, root: &Path) -> PathBuf {
    let root = root.to_string_lossy();
    let mut out = path.replace("$PROJECT_DIR$", &root);
    if out.contains("$USER_HOME$")
        && let Some(home) = dirs::home_dir()
    {
        out = out.replace("$USER_HOME$", &home.to_string_lossy());
    }
    PathBuf::from(out)
}

/// The reference's settings as Pyoxide's; `None` when they say nothing
/// about ruff, ty, basedpyright, Pyright or Pyrefly.
fn convert(tools: &ToolsFile, engine: Option<&str>, root: &Path) -> Option<Imported> {
    let mut entries = tools.entries();
    let enabled = |entries: &HashMap<String, Entry>, package: &str| {
        entries
            .get(package)
            .and_then(|e| e.enabled)
            .unwrap_or(false)
    };
    let mut notes = Vec::new();
    let mut any = false;

    // Pyright: basedpyright, its fork, has the closest semantics.
    if enabled(&entries, PYRIGHT) && !enabled(&entries, Tool::Basedpyright.package()) {
        let lookup = entries.get(PYRIGHT).and_then(|e| e.lookup);
        let entry = entries
            .entry(Tool::Basedpyright.package().to_owned())
            .or_default();
        entry.enabled = Some(true);
        entry.lookup = entry.lookup.or(lookup);
        notes.push(NOTE_PYRIGHT.to_owned());
        any = true;
    }
    let engine = engine.map(str::to_ascii_uppercase);
    let pyrefly_engine = engine.as_deref() == Some("PYREFLY");
    if pyrefly_engine || enabled(&entries, PYREFLY) {
        notes.push(NOTE_PYREFLY.to_owned());
        any = true;
    }
    let ty_engine = engine.as_deref() == Some("TY");
    let type_engine = if ty_engine || pyrefly_engine || enabled(&entries, Tool::Ty.package()) {
        TypeEngine::Ty
    } else if enabled(&entries, Tool::Basedpyright.package()) {
        TypeEngine::Basedpyright
    } else {
        TypeEngine::Ty
    };
    let mut settings = Settings::with_type_engine(type_engine);
    let mut paths = BTreeMap::new();
    for tool in Tool::ALL {
        let defaults = Settings::default_tool(tool, type_engine);
        let out = settings.tool_mut(tool);
        if let Some(entry) = entries.get(tool.package()) {
            if entry.enabled == Some(true) {
                out.enabled = true;
                out.explicit = true;
                any = true;
            }
            if let Some(lookup) = entry.lookup {
                out.lookup = lookup;
                any |= lookup != defaults.lookup;
            }
            if let Some(path) = entry.path.as_deref().filter(|p| !p.trim().is_empty()) {
                paths.insert(tool, expand(path.trim(), root));
                any = true;
            }
        }
        let mut flag = |name: &str, field: &mut bool| {
            if let Some(value) = tools.option(tool, name).and_then(parse_bool) {
                any |= value != *field;
                *field = value;
            }
        };
        flag("inspections", &mut out.inspections);
        if tool == Tool::Ruff {
            flag("formatting", &mut out.formatting);
            flag("sortImports", &mut out.import_optimizer);
        }
    }
    if ty_engine {
        settings.ty.enabled = true;
        settings.ty.explicit = true;
        any = true;
    }
    any.then_some(Imported {
        settings,
        paths,
        notes,
    })
}

/// Reads `pyLspTools.xml`.
fn parse_tools(text: &str) -> Result<ToolsFile, quick_xml::Error> {
    let mut out = ToolsFile::default();
    let mut component: Option<String> = None;
    let mut entry: Option<String> = None;
    walk(text, |depth, e| {
        let name = e.name();
        let name = name.as_ref();
        if depth == 1 && name == "component" {
            component = attr(e, "name");
            entry = None;
            if component.as_deref() == Some(TOOLS_STATE) {
                out.state.get_or_insert_default();
            }
            return;
        }
        let Some(component) = component.as_deref() else {
            return;
        };
        if component == TOOLS_STATE {
            if name == "entry" {
                entry = attr(e, "key");
            } else if name == "option"
                && let (Some(key), Some(state)) = (entry.as_deref(), out.state.as_mut())
                && let (Some(option), Some(value)) = (attr(e, "name"), attr(e, "value"))
            {
                let entry = state.entry(key.to_owned()).or_default();
                match option.as_str() {
                    "enabled" => entry.enabled = parse_bool(&value),
                    "discoveryMode" => entry.lookup = parse_mode(&value),
                    "customPathToExecutable" => entry.path = Some(value),
                    _ => {}
                }
            }
        } else if depth == 2
            && name == "option"
            && let (Some(option), Some(value)) = (attr(e, "name"), attr(e, "value"))
        {
            out.components
                .entry(component.to_owned())
                .or_default()
                .insert(option, value);
        }
    })?;
    Ok(out)
}

/// The `typeEngine` of `pyTypeEngine.xml`.
fn parse_type_engine(text: &str) -> Result<Option<String>, quick_xml::Error> {
    let mut in_settings = false;
    let mut out = None;
    walk(text, |depth, e| {
        let name = e.name();
        if depth == 1 && name.as_ref() == "component" {
            in_settings = attr(e, "name").as_deref() == Some(TYPE_ENGINE_SETTINGS);
        } else if in_settings
            && depth == 2
            && name.as_ref() == "option"
            && attr(e, "name").as_deref() == Some("typeEngine")
        {
            out = attr(e, "value");
        }
    })?;
    Ok(out)
}

/// Calls `visit` with each start tag of a document and its depth (the root
/// element at 0).
fn walk(text: &str, mut visit: impl FnMut(usize, &BytesStart<'_>)) -> Result<(), quick_xml::Error> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut depth = 0usize;
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                visit(depth, &e);
                depth += 1;
            }
            Event::Empty(e) => visit(depth, &e),
            Event::End(_) => depth = depth.saturating_sub(1),
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.as_ref() == name)
            .then(|| {
                a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(|v| v.into_owned())
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use project::workspace;

    use super::*;

    /// A file of an older version of the reference: everything in the
    /// per-tool components.
    const LEGACY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyrightConfiguration">
    <option name="executableDiscoveryMode" value="PATH" />
  </component>
  <component name="RuffConfiguration">
    <option name="enabled" value="true" />
    <option name="executableDiscoveryMode" value="PATH" />
  </component>
</project>
"#;

    /// The same project after the reference (2026.2.3) rewrote it.
    const MIGRATED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyToolsState">
    <option name="tools">
      <map>
        <entry key="pyright">
          <value>
            <ToolEntry>
              <option name="discoveryMode" value="PATH" />
            </ToolEntry>
          </value>
        </entry>
        <entry key="ruff">
          <value>
            <ToolEntry>
              <option name="enabled" value="true" />
              <option name="discoveryMode" value="PATH" />
            </ToolEntry>
          </value>
        </entry>
      </map>
    </option>
  </component>
</project>
"#;

    /// The current form with every field: custom paths, features, and
    /// leftovers of the older form that no longer count.
    const CURRENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="BasedpyrightConfiguration">
    <option name="inspections" value="false" />
  </component>
  <component name="PyToolsState">
    <option name="tools">
      <map>
        <entry key="basedpyright">
          <value>
            <ToolEntry>
              <option name="enabled" value="true" />
              <option name="customPathToExecutable" value="$PROJECT_DIR$/tools/basedpyright-langserver" />
            </ToolEntry>
          </value>
        </entry>
        <entry key="ruff">
          <value>
            <ToolEntry>
              <option name="enabled" value="true" />
              <option name="discoveryMode" value="PATH" />
            </ToolEntry>
          </value>
        </entry>
        <entry key="ty">
          <value>
            <ToolEntry>
              <option name="discoveryMode" value="UVX" />
            </ToolEntry>
          </value>
        </entry>
      </map>
    </option>
  </component>
  <component name="RuffConfiguration">
    <option name="executableDiscoveryMode" value="UVX" />
    <option name="formatting" value="false" />
    <option name="sortImports" value="false" />
  </component>
</project>
"#;

    const TY_ENGINE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PyTypeEngineProjectSettings">
    <option name="typeEngine" value="TY" />
  </component>
</project>
"#;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let idea = dir.path().join(IDEA_DIR);
        std::fs::create_dir_all(&idea).unwrap();
        for (name, text) in files {
            std::fs::write(idea.join(name), text).unwrap();
        }
        dir
    }

    fn ruff_on_path() -> Settings {
        let mut settings = Settings::default();
        settings.ruff.explicit = true;
        settings.ruff.lookup = Lookup::Path;
        settings
    }

    #[test]
    fn the_older_form_is_migrated() {
        let dir = project(&[(TOOLS_FILE, LEGACY)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        assert_eq!(imported.settings, ruff_on_path());
        assert!(imported.paths.is_empty());
    }

    #[test]
    fn the_current_form() {
        let dir = project(&[(TOOLS_FILE, MIGRATED)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        assert_eq!(imported.settings, ruff_on_path());

        let dir = project(&[(TOOLS_FILE, CURRENT)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        let s = &imported.settings;
        assert_eq!(s.type_engine, TypeEngine::Basedpyright);
        assert_eq!(s.servers(), [Tool::Ruff, Tool::Basedpyright]);
        // PyToolsState wins over the older fields.
        assert_eq!(s.ruff.lookup, Lookup::Path);
        assert!(s.ruff.explicit && !s.ruff.formatting && !s.ruff.import_optimizer);
        assert!(s.ruff.inspections);
        assert!(s.basedpyright.explicit && !s.basedpyright.inspections);
        assert_eq!(s.ty.lookup, Lookup::Uvx);
        assert!(!s.ty.enabled && !s.ty.explicit);
        assert_eq!(
            imported.paths,
            BTreeMap::from([(
                Tool::Basedpyright,
                dir.path().join("tools").join("basedpyright-langserver")
            )])
        );
    }

    #[test]
    fn the_type_engine() {
        // ty as the reference's type engine wins over basedpyright.
        let dir = project(&[(TOOLS_FILE, CURRENT), (TYPE_ENGINE_FILE, TY_ENGINE)]);
        let s = read_tool_settings(dir.path()).unwrap().settings;
        assert_eq!(s.type_engine, TypeEngine::Ty);
        assert_eq!(s.servers(), [Tool::Ruff, Tool::Ty]);
        assert!(s.ty.explicit);

        let dir = project(&[(TYPE_ENGINE_FILE, TY_ENGINE)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        assert!(imported.settings.ty.explicit && imported.settings.ty.uvx_without_asking());
        assert!(imported.notes.is_empty());

        // The built-in engine has no counterpart.
        let text = TY_ENGINE.replace("TY", "PYCHARM");
        let dir = project(&[(TYPE_ENGINE_FILE, &text)]);
        assert_eq!(read_tool_settings(dir.path()), None);

        // Pyrefly is not supported yet: ty, with a note, even next to an
        // enabled basedpyright.
        let text = TY_ENGINE.replace("TY", "PYREFLY");
        let dir = project(&[(TOOLS_FILE, CURRENT), (TYPE_ENGINE_FILE, &text)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        assert_eq!(imported.settings.type_engine, TypeEngine::Ty);
        assert_eq!(imported.settings.servers(), [Tool::Ruff, Tool::Ty]);
        assert!(!imported.settings.ty.explicit);
        assert_eq!(imported.notes, [NOTE_PYREFLY]);
        let dir = project(&[(TYPE_ENGINE_FILE, &text)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        assert_eq!(imported.settings, Settings::default());
        assert_eq!(imported.notes, [NOTE_PYREFLY]);
    }

    #[test]
    fn pyright_becomes_basedpyright() {
        let pyright = r#"<project version="4">
  <component name="PyToolsState"><option name="tools"><map>
    <entry key="pyright"><value><ToolEntry>
      <option name="enabled" value="true" />
      <option name="discoveryMode" value="UVX" />
      <option name="customPathToExecutable" value="/opt/node/bin/pyright-langserver" />
    </ToolEntry></value></entry>
  </map></option></component>
</project>"#;
        let dir = project(&[(TOOLS_FILE, pyright)]);
        let imported = read_tool_settings(dir.path()).unwrap();
        let s = &imported.settings;
        assert_eq!(s.type_engine, TypeEngine::Basedpyright);
        assert_eq!(s.servers(), [Tool::Ruff, Tool::Basedpyright]);
        assert!(s.basedpyright.explicit);
        assert_eq!(s.basedpyright.lookup, Lookup::Uvx);
        // Pyright's own executable is not basedpyright's.
        assert!(imported.paths.is_empty());
        assert_eq!(imported.notes, [NOTE_PYRIGHT]);

        // The older form, and ty enabled next to it: ty is the engine.
        let legacy = r#"<project version="4">
  <component name="PyrightConfiguration"><option name="enabled" value="true" /></component>
  <component name="TyConfiguration"><option name="enabled" value="true" /></component>
</project>"#;
        let dir = project(&[(TOOLS_FILE, legacy)]);
        let s = read_tool_settings(dir.path()).unwrap().settings;
        assert_eq!(s.type_engine, TypeEngine::Ty);
        assert!(s.ty.explicit && s.basedpyright.explicit);
        assert_eq!(s.servers(), [Tool::Ruff, Tool::Ty]);

        // Pyright only at its defaults: nothing.
        let dir = project(&[(
            TOOLS_FILE,
            LEGACY.replace("RuffConfiguration", "Other").as_str(),
        )]);
        assert_eq!(read_tool_settings(dir.path()), None);
    }

    #[test]
    fn nothing_to_import() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_tool_settings(dir.path()), None);
        // Defaults only.
        let defaults = r#"<project version="4">
  <component name="PyToolsState"><option name="tools"><map>
    <entry key="ruff"><value><ToolEntry><option name="discoveryMode" value="INTERPRETER" /></ToolEntry></value></entry>
    <entry key="black"><value><ToolEntry><option name="enabled" value="true" /></ToolEntry></value></entry>
  </map></option></component>
  <component name="RuffConfiguration"><option name="inspections" value="true" /></component>
</project>"#;
        let dir = project(&[(TOOLS_FILE, defaults)]);
        assert_eq!(read_tool_settings(dir.path()), None);
        assert_eq!(import_tool_settings(dir.path()).unwrap(), None);
        assert!(!store::settings_file(dir.path()).exists());
        // Broken files are skipped.
        let dir = project(&[
            (TOOLS_FILE, "<project><component name="),
            (TYPE_ENGINE_FILE, ""),
        ]);
        assert_eq!(read_tool_settings(dir.path()), None);
    }

    #[test]
    fn an_import_of_a_path_only_happens_once() {
        let only_path = r#"<project version="4">
  <component name="PyToolsState"><option name="tools"><map>
    <entry key="ruff"><value><ToolEntry>
      <option name="customPathToExecutable" value="$PROJECT_DIR$/bin/ruff" />
    </ToolEntry></value></entry>
  </map></option></component>
</project>"#;
        let dir = project(&[(TOOLS_FILE, only_path)]);
        let root = dir.path();
        let imported = import_tool_settings(root).unwrap().unwrap();
        assert_eq!(imported.settings, Settings::default());
        let text = std::fs::read_to_string(store::settings_file(root)).unwrap();
        assert!(text.contains("[python.tools]\n"), "{text}");
        assert!(!text.contains("imported"), "{text}");
        assert!(settings::has_tool_settings(root).unwrap());
        assert_eq!(settings::load(root).unwrap(), Settings::default());
        // The person removes the path: it does not come back.
        settings::set_custom_path(root, Tool::Ruff, None).unwrap();
        assert_eq!(import_tool_settings(root).unwrap(), None);
        assert_eq!(settings::custom_path(root, Tool::Ruff).unwrap(), None);
    }

    #[test]
    fn imported_once_and_idea_is_not_touched() {
        let dir = project(&[(TOOLS_FILE, CURRENT)]);
        let root = dir.path();
        let imported = import_tool_settings(root).unwrap().unwrap();
        assert_eq!(settings::load(root).unwrap(), imported.settings);
        assert_eq!(
            settings::custom_path(root, Tool::Basedpyright).unwrap(),
            Some(root.join("tools").join("basedpyright-langserver"))
        );
        let text = std::fs::read_to_string(store::settings_file(root)).unwrap();
        assert!(
            text.contains("[python.tools.ruff]\nenabled = true\nlookup = \"path\"\n"),
            "{text}"
        );
        assert!(text.contains("name = \"basedpyright\""), "{text}");
        let personal = std::fs::read_to_string(workspace::workspace_file(root)).unwrap();
        assert!(
            personal.contains("[python.tools.basedpyright]"),
            "{personal}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(IDEA_DIR).join(TOOLS_FILE)).unwrap(),
            CURRENT
        );

        // Later changes in `.idea` are not imported again.
        std::fs::write(root.join(IDEA_DIR).join(TYPE_ENGINE_FILE), TY_ENGINE).unwrap();
        assert_eq!(import_tool_settings(root).unwrap(), None);
        assert_eq!(settings::load(root).unwrap(), imported.settings);
    }

    #[test]
    fn after_the_exclusions_import() {
        let dir = project(&[(TOOLS_FILE, LEGACY)]);
        let root = dir.path();
        std::fs::write(
            root.join(IDEA_DIR).join("demo.iml"),
            r#"<module type="PYTHON_MODULE" version="4">
  <component name="NewModuleRootManager">
    <content url="file://$MODULE_DIR$">
      <excludeFolder url="file://$MODULE_DIR$/.venv" />
    </content>
  </component>
</module>"#,
        )
        .unwrap();
        // The order of opening a project: exclusions first, then tools.
        assert!(store::import_idea(root).unwrap().is_some());
        assert!(import_tool_settings(root).unwrap().is_some());
        let text = std::fs::read_to_string(store::settings_file(root)).unwrap();
        assert!(text.contains("[exclusions]"), "{text}");
        assert!(text.contains("[python.tools.ruff]"), "{text}");
        assert_eq!(settings::load(root).unwrap(), ruff_on_path());
    }

    #[test]
    fn a_type_engine_chosen_in_pyoxide_stays() {
        let dir = project(&[(TOOLS_FILE, MIGRATED), (TYPE_ENGINE_FILE, TY_ENGINE)]);
        let root = dir.path();
        store::save_values(
            root,
            settings::TYPE_ENGINE_TABLE,
            &[("name", Some(toml::Value::from("basedpyright")))],
        )
        .unwrap();
        let imported = import_tool_settings(root).unwrap().unwrap();
        assert_eq!(imported.settings.type_engine, TypeEngine::Basedpyright);
        let s = settings::load(root).unwrap();
        assert_eq!(s.type_engine, TypeEngine::Basedpyright);
        assert_eq!(s.servers(), [Tool::Ruff, Tool::Basedpyright]);
        assert!(s.ruff.explicit);
    }

    #[test]
    fn macros_in_paths() {
        let root = Path::new("/work/demo");
        assert_eq!(
            expand("$PROJECT_DIR$/bin/ruff", root),
            PathBuf::from("/work/demo/bin/ruff")
        );
        assert_eq!(expand("/opt/ruff", root), PathBuf::from("/opt/ruff"));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(
                expand("$USER_HOME$/.local/bin/ty", root),
                PathBuf::from(format!("{}/.local/bin/ty", home.to_string_lossy()))
            );
        }
    }
}
