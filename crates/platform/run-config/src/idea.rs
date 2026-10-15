//! Reads the run configurations of a project of the reference IDE from its
//! `.idea` folder and from `*.run.xml` files, for the one-time import into
//! Pyoxide's storage (ADR 0012). Nothing is ever written into `.idea`.
//!
//! - Per-user configurations are in `.idea/workspace.xml`, component
//!   `RunManager` (`RunManagerImpl`): the `<configuration>` elements, the
//!   `selected` attribute (a unique id, `"<type display name>.<name>"`),
//!   the custom order (`<list><item itemvalue/>`) and the recently used
//!   temporary configurations (`<recent_temporary>`). The last executor of
//!   each configuration is a `<unique id>.executor` property of
//!   `PropertiesComponent`.
//! - Shared configurations are in `.idea/runConfigurations/*.xml` and in
//!   `*.run.xml` files of the project, each wrapped in
//!   `<component name="ProjectRunConfigurationManager">`
//!   (`RunConfigurationSchemeManager`, `RCInArbitraryFileManager`).
//!
//! A `<configuration>` has `name`, `type`, `factoryName` and optionally
//! `folderName`, `temporary` (personal ones only), `singleton`,
//! `editBeforeRun`, `activateToolWindowBeforeRun`,
//! `focusToolWindowBeforeRun`, `nameIsGenerated`; `default="true"` marks a
//! template, which is skipped. Before-launch tasks are the options of
//! `<method v="2">` (`RunnerAndConfigurationSettingsImpl`,
//! `RunManagerImpl.readBeforeRunTasks`). The options of each type are read
//! by an [`Importer`]; configurations of a type no importer converts are
//! kept read-only with their element as it was written.

use std::path::{Path, PathBuf};

use crate::model::{BeforeLaunchTask, Reference, RunConfiguration, Storage, UNSUPPORTED_PREFIX};
use crate::xml::{self, Element};

/// Name of the reference IDE's project folder.
pub const IDEA_DIR: &str = ".idea";
/// The shared configurations' folder in `.idea`.
pub const SHARED_DIR: &str = "runConfigurations";
/// The suffix of configuration files anywhere in the project.
pub const RUN_XML: &str = ".run.xml";

/// How deep and how far the search for `*.run.xml` files goes.
const MAX_DEPTH: usize = 24;
const MAX_ENTRIES: usize = 200_000;

/// A `<configuration>` element of the reference and the document it is in.
#[derive(Debug, Clone, Copy)]
pub struct IdeaConfiguration<'a> {
    pub element: &'a Element,
    /// The whole document (for the element as written).
    pub document: &'a str,
}

impl<'a> IdeaConfiguration<'a> {
    pub fn name(&self) -> &'a str {
        self.element.attr("name").unwrap_or("")
    }

    pub fn type_id(&self) -> &'a str {
        self.element.attr("type").unwrap_or("")
    }

    pub fn factory(&self) -> Option<&'a str> {
        self.element.attr("factoryName")
    }

    /// A field (`<option name="X" value="Y"/>`).
    pub fn field(&self, name: &str) -> Option<&'a str> {
        self.element.field(name)
    }

    /// A boolean field; `default` when it is missing or empty.
    pub fn bool_field(&self, name: &str, default: bool) -> bool {
        match self.field(name) {
            Some(v) if !v.is_empty() => v.eq_ignore_ascii_case("true"),
            _ => default,
        }
    }

    /// The variables of `<envs><env name value/></envs>`, in order.
    pub fn envs(&self) -> Vec<(String, String)> {
        let Some(envs) = self.element.child("envs") else {
            return Vec::new();
        };
        envs.children_named("env")
            .filter_map(|e| Some((e.attr("name")?.to_owned(), e.attr("value")?.to_owned())))
            .collect()
    }

    /// The element exactly as it is written.
    pub fn xml(&self) -> &'a str {
        self.element.source(self.document)
    }
}

/// Converts the reference's configurations of some types into Pyoxide's
/// (the platform converts Shell Script, a product its language's types).
pub trait Importer: Send + Sync {
    /// The reference's display name of a type this importer knows
    /// (`Python`, `Python tests`), used in the reference's unique ids and
    /// to list a configuration of an unsupported type.
    fn type_name(&self, reference_type: &str) -> Option<&'static str>;

    /// Fills `config`, whose common settings are read already (name,
    /// folder, storage, flags, before-launch tasks), with a supported type
    /// and its options. Returns false to leave the configuration to
    /// another importer, or to keep it read-only.
    fn convert(
        &self,
        idea: &IdeaConfiguration<'_>,
        root: &Path,
        config: &mut RunConfiguration,
    ) -> bool;
}

/// Names of reference types no product converts, so that their
/// configurations are listed with their type's name.
fn platform_type_name(type_id: &str) -> Option<&'static str> {
    Some(match type_id {
        "CompoundRunConfigurationType" => "Compound",
        "docker-deploy" => "Docker",
        "ShConfigurationType" => "Shell Script",
        _ => return None,
    })
}

/// A configuration read from the reference with its unique id there.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// `"<type display name>.<name>"` (`RunnerAndConfigurationSettingsImpl.uniqueID`).
    pub unique_id: String,
    pub config: RunConfiguration,
}

/// The per-user state of `workspace.xml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Workspace {
    pub configurations: Vec<Imported>,
    /// The selected configuration's unique id.
    pub selected: Option<String>,
    /// The custom order, by unique id.
    pub order: Vec<String>,
    /// Recently used temporary configurations, by unique id.
    pub recent_temporary: Vec<String>,
    /// The last executor by unique id (`Run`, `Debug`).
    pub executors: Vec<(String, String)>,
}

/// What the reference has for a project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IdeaImport {
    /// `.idea/runConfigurations` first (by file name), then `*.run.xml`.
    pub shared: Vec<Imported>,
    pub workspace: Option<Workspace>,
}

impl IdeaImport {
    pub fn is_empty(&self) -> bool {
        self.shared.is_empty()
            && self
                .workspace
                .as_ref()
                .is_none_or(|w| w.configurations.is_empty())
    }
}

/// Reads the reference's configurations of the project at `root`: the
/// shared ones when `shared`, the per-user ones (and the selection, order,
/// recent list and executors) when `personal`. Blocking: run it off the
/// UI thread. Unreadable files are skipped.
pub fn read(root: &Path, importers: &[&dyn Importer], shared: bool, personal: bool) -> IdeaImport {
    let idea = root.join(IDEA_DIR);
    let mut out = IdeaImport::default();
    if shared {
        let mut files: Vec<PathBuf> = std::fs::read_dir(idea.join(SHARED_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "xml") && p.is_file())
            .collect();
        files.sort();
        files.extend(run_xml_files(root));
        for file in files {
            if let Ok(text) = std::fs::read_to_string(&file) {
                out.shared
                    .extend(read_shared_document(&text, root, importers));
            }
        }
    }
    if personal && let Ok(text) = std::fs::read_to_string(idea.join("workspace.xml")) {
        out.workspace = read_workspace(&text, root, importers);
    }
    out
}

/// The configurations of a shared file (`ProjectRunConfigurationManager`).
pub fn read_shared_document(text: &str, root: &Path, importers: &[&dyn Importer]) -> Vec<Imported> {
    let Ok(doc) = xml::parse(text) else {
        return Vec::new();
    };
    let component = if doc.name == "component" {
        Some(&doc)
    } else {
        doc.children_named("component")
            .find(|c| c.attr("name") == Some("ProjectRunConfigurationManager"))
    };
    let Some(component) = component else {
        return Vec::new();
    };
    component
        .children_named("configuration")
        .filter_map(|e| {
            configuration(
                IdeaConfiguration {
                    element: e,
                    document: text,
                },
                Storage::Shared,
                root,
                importers,
            )
        })
        .collect()
}

/// The `RunManager` state of `workspace.xml`; `None` without one.
pub fn read_workspace(text: &str, root: &Path, importers: &[&dyn Importer]) -> Option<Workspace> {
    let doc = xml::parse(text).ok()?;
    let component = |name: &str| {
        doc.children_named("component")
            .find(|c| c.attr("name") == Some(name))
    };
    let manager = component("RunManager")?;
    let mut out = Workspace::default();
    let mut old_style_selected = None;
    for e in manager
        .children
        .iter()
        .filter(|c| c.name == "configuration" || c.name == "tempConfiguration")
    {
        let idea = IdeaConfiguration {
            element: e,
            document: text,
        };
        if let Some(imported) = configuration(idea, Storage::Personal, root, importers) {
            if e.bool_attr("selected") && old_style_selected.is_none() {
                old_style_selected = Some(imported.unique_id.clone());
            }
            out.configurations.push(imported);
        }
    }
    out.selected = manager
        .attr("selected")
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or(old_style_selected);
    let items = |list: &Element| -> Vec<String> {
        list.children_named("item")
            .filter_map(|i| i.attr("itemvalue").map(str::to_owned))
            .collect()
    };
    if let Some(list) = manager.child("list") {
        out.order = items(list);
    }
    if let Some(list) = manager
        .child("recent_temporary")
        .and_then(|r| r.child("list"))
    {
        out.recent_temporary = items(list);
    }
    if let Some(properties) = component("PropertiesComponent") {
        out.executors = executors(properties);
    }
    Some(out)
}

/// `<unique id>.executor` properties (`ExecutionManagerImpl.registerRecentExecutor`):
/// the JSON `keyToString` map the reference writes as the component's
/// text, or `<property name value/>` elements of older versions.
fn executors(properties: &Element) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = properties
        .children_named("property")
        .filter_map(|p| Some((p.attr("name")?.to_owned(), p.attr("value")?.to_owned())))
        .collect();
    if let Ok(serde_json::Value::Object(map)) =
        serde_json::from_str::<serde_json::Value>(properties.text.trim())
        && let Some(serde_json::Value::Object(keys)) = map.get("keyToString")
    {
        for (k, v) in keys {
            if let Some(v) = v.as_str() {
                pairs.push((k.clone(), v.to_owned()));
            }
        }
    }
    pairs
        .into_iter()
        .filter_map(|(k, v)| Some((k.strip_suffix(".executor")?.to_owned(), v)))
        .collect()
}

/// One `<configuration>`: its common settings, then its type's options.
/// `None` for a template or a configuration without a name or type.
fn configuration(
    idea: IdeaConfiguration<'_>,
    storage: Storage,
    root: &Path,
    importers: &[&dyn Importer],
) -> Option<Imported> {
    let e = idea.element;
    if e.bool_attr("default") {
        return None;
    }
    let name = idea.name();
    let type_id = idea.type_id();
    if name.is_empty() || type_id.is_empty() {
        return None;
    }
    let mut config = RunConfiguration::new("", name, storage);
    config.folder = e
        .attr("folderName")
        .filter(|f| !f.is_empty())
        .map(str::to_owned);
    // Shared configurations are never temporary.
    config.temporary =
        storage == Storage::Personal && (e.bool_attr("temporary") || e.name == "tempConfiguration");
    config.edit_before_run = e.bool_attr("editBeforeRun");
    config.activate_tool_window = e
        .attr("activateToolWindowBeforeRun")
        .is_none_or(|v| v.eq_ignore_ascii_case("true"));
    config.focus_tool_window = e.bool_attr("focusToolWindowBeforeRun");
    config.allow_parallel = e
        .attr("singleton")
        .filter(|s| !s.is_empty())
        .map(|s| !s.eq_ignore_ascii_case("true"));
    config.name_is_generated = e.bool_attr("nameIsGenerated");
    if let Some(method) = e.child("method") {
        config.before_launch = method
            .children_named("option")
            .filter_map(|o| {
                Some(BeforeLaunchTask {
                    provider: o.attr("name")?.to_owned(),
                    enabled: o.attr("enabled") != Some("false"),
                    xml: o.source(idea.document).to_owned(),
                })
            })
            .collect();
    }
    let type_name = importers
        .iter()
        .find_map(|i| i.type_name(type_id))
        .or_else(|| platform_type_name(type_id));
    let unique_id = format!("{}.{name}", type_name.unwrap_or(type_id));
    let converted = importers
        .iter()
        .any(|i| i.convert(&idea, root, &mut config));
    if !converted {
        config.type_id = format!("{UNSUPPORTED_PREFIX}{type_id}");
        config.reference = Some(Reference {
            type_id: type_id.to_owned(),
            factory: idea.factory().map(str::to_owned),
            type_name: type_name.map(str::to_owned),
            xml: idea.xml().to_owned(),
        });
    }
    Some(Imported { unique_id, config })
}

/// The `*.run.xml` files of the project: its folders that are shown and
/// not excluded (the project's rules, ADR 0008), without `.idea`.
fn run_xml_files(root: &Path) -> Vec<PathBuf> {
    let filter = project::Filter::for_root(root);
    let mut out = Vec::new();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_owned(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            seen += 1;
            if seen > MAX_ENTRIES {
                return out;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if filter.is_hidden(&name) || name == IDEA_DIR {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !filter.is_excluded(&path, true) {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file() && name.ends_with(RUN_XML) && !filter.is_excluded(&path, false)
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Converts only `Known` configurations: the name and one field.
    struct KnownImporter;

    impl Importer for KnownImporter {
        fn type_name(&self, reference_type: &str) -> Option<&'static str> {
            (reference_type == "Known").then_some("Known Type")
        }

        fn convert(
            &self,
            idea: &IdeaConfiguration<'_>,
            _: &Path,
            config: &mut RunConfiguration,
        ) -> bool {
            if idea.type_id() != "Known" {
                return false;
            }
            config.type_id = "known".into();
            config.set_str("value", idea.field("VALUE").unwrap_or(""));
            true
        }
    }

    const WORKSPACE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PropertiesComponent">{
  &quot;keyToString&quot;: {
    &quot;Known Type.first.executor&quot;: &quot;Debug&quot;,
    &quot;Known Type.gone.executor&quot;: &quot;Run&quot;,
    &quot;other.key&quot;: &quot;x&quot;
  }
}</component>
  <component name="RunManager" selected="Known Type.second">
    <configuration name="first" type="Known" factoryName="Known" folderName="Group" singleton="false" editBeforeRun="true" activateToolWindowBeforeRun="false" focusToolWindowBeforeRun="true" nameIsGenerated="true">
      <option name="VALUE" value="1" />
      <method v="2">
        <option name="Make" enabled="false" />
        <option name="RunConfigurationTask" run_configuration_name="second" run_configuration_type="Known" />
      </method>
    </configuration>
    <configuration name="second" type="Known" factoryName="Known" temporary="true">
      <option name="VALUE" value="2" />
      <method v="2" />
    </configuration>
    <configuration default="true" type="Known" factoryName="Known">
      <option name="VALUE" value="template" />
      <method v="2" />
    </configuration>
    <configuration default="true" type="docker-deploy" factoryName="docker-compose.yml" temporary="true">
      <deployment type="docker-compose.yml" />
      <method v="2" />
    </configuration>
    <configuration name="mystery" type="SomethingElse" factoryName="X">
      <option name="SECRET_OPTION" value="kept &amp; verbatim" />
      <method v="2" />
    </configuration>
    <list>
      <item itemvalue="Known Type.second" />
      <item itemvalue="Known Type.first" />
    </list>
    <recent_temporary>
      <list>
        <item itemvalue="Known Type.second" />
      </list>
    </recent_temporary>
  </component>
</project>
"#;

    #[test]
    fn workspace_state_is_read() {
        let root = Path::new("/work/demo");
        let ws = read_workspace(WORKSPACE, root, &[&KnownImporter]).unwrap();
        let ids: Vec<&str> = ws
            .configurations
            .iter()
            .map(|c| c.unique_id.as_str())
            .collect();
        // Templates are skipped, an unknown type is kept.
        assert_eq!(
            ids,
            [
                "Known Type.first",
                "Known Type.second",
                "SomethingElse.mystery"
            ]
        );
        assert_eq!(ws.selected.as_deref(), Some("Known Type.second"));
        assert_eq!(ws.order, ["Known Type.second", "Known Type.first"]);
        assert_eq!(ws.recent_temporary, ["Known Type.second"]);
        assert_eq!(
            ws.executors,
            [
                ("Known Type.first".to_owned(), "Debug".to_owned()),
                ("Known Type.gone".to_owned(), "Run".to_owned())
            ]
        );
        let first = &ws.configurations[0].config;
        assert_eq!(first.type_id, "known");
        assert_eq!(first.str_option("value"), "1");
        assert_eq!(first.folder.as_deref(), Some("Group"));
        assert_eq!(first.storage, Storage::Personal);
        assert!(!first.temporary);
        assert_eq!(first.allow_parallel, Some(true));
        assert!(first.edit_before_run && first.focus_tool_window && first.name_is_generated);
        assert!(!first.activate_tool_window);
        assert_eq!(first.before_launch.len(), 2);
        assert_eq!(first.before_launch[0].provider, "Make");
        assert!(!first.before_launch[0].enabled);
        assert!(first.before_launch[1].enabled);
        assert_eq!(
            first.before_launch[1].xml,
            r#"<option name="RunConfigurationTask" run_configuration_name="second" run_configuration_type="Known" />"#
        );
        let second = &ws.configurations[1].config;
        assert!(second.temporary);
        assert_eq!(second.allow_parallel, None);
        assert!(second.activate_tool_window);
        let mystery = &ws.configurations[2].config;
        assert!(mystery.is_unsupported());
        assert_eq!(mystery.type_id, "idea:SomethingElse");
        let reference = mystery.reference.as_ref().unwrap();
        assert_eq!(reference.type_name, None);
        assert_eq!(reference.factory.as_deref(), Some("X"));
        assert!(reference.xml.starts_with("<configuration name=\"mystery\""));
        assert!(reference.xml.contains("kept &amp; verbatim"));
        assert!(reference.xml.ends_with("</configuration>"));
    }

    #[test]
    fn shared_files_are_never_temporary_and_old_selection_counts() {
        let shared = r#"<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="one" type="Known" factoryName="Known" temporary="true">
    <option name="VALUE" value="x" />
  </configuration>
  <configuration name="two" type="CompoundRunConfigurationType">
    <toRun name="one" type="Known" />
  </configuration>
</component>"#;
        let configs = read_shared_document(shared, Path::new("/p"), &[&KnownImporter]);
        assert_eq!(configs.len(), 2);
        assert!(!configs[0].config.temporary);
        assert_eq!(configs[0].config.storage, Storage::Shared);
        assert_eq!(configs[1].unique_id, "Compound.two");
        assert_eq!(
            configs[1]
                .config
                .reference
                .as_ref()
                .unwrap()
                .type_name
                .as_deref(),
            Some("Compound")
        );
        let old = r#"<project><component name="RunManager"><configuration name="a" type="Known" selected="true"/></component></project>"#;
        let ws = read_workspace(old, Path::new("/p"), &[&KnownImporter]).unwrap();
        assert_eq!(ws.selected.as_deref(), Some("Known Type.a"));
        assert!(read_workspace("<project/>", Path::new("/p"), &[]).is_none());
        assert!(read_shared_document("<broken", Path::new("/p"), &[]).is_empty());
    }

    #[test]
    fn run_xml_files_are_found_outside_excluded_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = |rel: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                "<component name=\"ProjectRunConfigurationManager\"/>",
            )
            .unwrap();
        };
        file(".run/server.run.xml");
        file("tools/build.run.xml");
        file(".venv/lib/x.run.xml");
        file(".idea/ignored.run.xml");
        file("node.xml");
        let found: Vec<String> = run_xml_files(root)
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        // `.venv` is excluded only when the product says so; the platform
        // default excludes nothing but the ignored names.
        assert!(
            found.contains(&".run/server.run.xml".to_owned()),
            "{found:?}"
        );
        assert!(
            found.contains(&"tools/build.run.xml".to_owned()),
            "{found:?}"
        );
        assert!(!found.iter().any(|f| f.starts_with(".idea")), "{found:?}");
        assert!(!found.iter().any(|f| f == "node.xml"), "{found:?}");
    }
}
