//! Reads the exclusions of a project of the reference IDE from its `.idea`
//! folder, for the one-time import into Pyoxide's project settings
//! (ADR 0008). Nothing is ever written into `.idea`.
//!
//! `.idea/modules.xml` lists the module files of the project:
//!
//! ```xml
//! <component name="ProjectModuleManager">
//!   <modules>
//!     <module fileurl="file://$PROJECT_DIR$/.idea/demo.iml"
//!             filepath="$PROJECT_DIR$/.idea/demo.iml" />
//!   </modules>
//! </component>
//! ```
//!
//! Each module file has content roots with excluded folders and exclude
//! patterns:
//!
//! ```xml
//! <component name="NewModuleRootManager">
//!   <content url="file://$MODULE_DIR$">
//!     <excludeFolder url="file://$MODULE_DIR$/.venv" />
//!     <excludePattern pattern="*.egg-info" />
//!   </content>
//! </component>
//! ```
//!
//! `$PROJECT_DIR$` is the folder that contains `.idea`; `$MODULE_DIR$` is the
//! folder of the module file, or the project folder when the module file is
//! inside `.idea`. Without `modules.xml` the module files directly in `.idea`
//! are read. Excluded folders outside the project are dropped; exclude
//! patterns are taken from the content root at the project folder only.
//!
//! A module names its SDK with `<orderEntry type="jdk" jdkName="…"
//! jdkType="…"/>`, or inherits the project's (`inheritedJdk`), which
//! `.idea/misc.xml` names: `<component name="ProjectRootManager"
//! project-jdk-name="…" project-jdk-type="…"/>`. The SDKs themselves are in
//! the IDE's SDK table, `options/jdk.table.xml` of its configuration
//! directory ([`read_sdk_table`]), for the interpreter import of ADR 0009.

use std::path::{Path, PathBuf};

use quick_xml::events::{BytesStart, Event};

use crate::store::{Exclusions, lexical, relative_to};

/// Name of the reference IDE's project folder.
pub const IDEA_DIR: &str = ".idea";

/// The exclusions of the reference IDE's project in `root`; `None` when
/// there is no `.idea` folder. Unreadable module files are skipped.
pub fn read_exclusions(root: &Path) -> Option<Exclusions> {
    let idea = root.join(IDEA_DIR);
    if !idea.is_dir() {
        return None;
    }
    let mut out = Exclusions::default();
    for module in module_files(root, &idea) {
        let Ok(text) = std::fs::read_to_string(&module) else {
            continue;
        };
        let module_dir = module_dir(&module);
        let Ok(module) = parse_module(&text) else {
            continue;
        };
        let expand = |url: &str| url_to_path(url, root, &module_dir);
        for content in module.contents {
            let Some(content_dir) = expand(&content.url) else {
                continue;
            };
            for folder in content.exclude_folders {
                if let Some(rel) = expand(&folder).and_then(|p| relative_to(root, &p)) {
                    out.paths.insert(rel);
                }
            }
            if lexical(&content_dir) == lexical(root) {
                for pattern in content.exclude_patterns {
                    out.add_pattern(&pattern);
                }
            }
        }
    }
    Some(out)
}

/// The module files the project lists, or the `.iml` files in `.idea`.
fn module_files(root: &Path, idea: &Path) -> Vec<PathBuf> {
    if let Ok(text) = std::fs::read_to_string(idea.join("modules.xml"))
        && let Ok(listed) = parse_modules(&text)
    {
        return listed
            .into_iter()
            .filter_map(|m| expand_macros(&m, root, root))
            .collect();
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(idea)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "iml") && p.is_file())
        .collect();
    files.sort();
    files
}

/// `$MODULE_DIR$` of a module file.
fn module_dir(module_file: &Path) -> PathBuf {
    let dir = module_file.parent().unwrap_or(Path::new(""));
    match dir.parent() {
        Some(parent) if dir.file_name().is_some_and(|n| n == IDEA_DIR) => parent.to_owned(),
        _ => dir.to_owned(),
    }
}

/// A `file://` URL of a module file as a path; other schemes (`jar://`)
/// and unknown path variables give `None`.
fn url_to_path(url: &str, project_dir: &Path, module_dir: &Path) -> Option<PathBuf> {
    expand_macros(url.strip_prefix("file://")?, project_dir, module_dir)
}

/// Expands `$PROJECT_DIR$`, `$MODULE_DIR$` and `$USER_HOME$` at the start
/// of a path written with `/`.
fn expand_macros(path: &str, project_dir: &Path, module_dir: &Path) -> Option<PathBuf> {
    let home =
        || std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let (base, rest) = if let Some(rest) = path.strip_prefix("$PROJECT_DIR$") {
        (project_dir.to_owned(), rest)
    } else if let Some(rest) = path.strip_prefix("$MODULE_DIR$") {
        (module_dir.to_owned(), rest)
    } else if let Some(rest) = path.strip_prefix("$USER_HOME$") {
        (home()?, rest)
    } else if path.starts_with('$') {
        return None;
    } else {
        // An absolute path: `/x/y` or `C:/x/y`.
        return Some(lexical(Path::new(path)));
    };
    if !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    let mut out = base;
    for part in rest.split('/').filter(|p| !p.is_empty()) {
        out.push(part);
    }
    Some(lexical(&out))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Module {
    contents: Vec<Content>,
    /// The SDK of the module: `Some(None)` inherits the project's.
    sdk: Option<Option<SdkRef>>,
}

/// An SDK as a project names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkRef {
    pub name: String,
    /// The SDK type (`Python SDK`).
    pub type_id: String,
}

/// An SDK of the IDE's SDK table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkEntry {
    pub name: String,
    pub type_id: String,
    /// The version string (`Python 3.13.1`).
    pub version: Option<String>,
    /// The home path (for a Python SDK its interpreter), `$USER_HOME$`
    /// expanded; `None` when it uses another path variable.
    pub home: Option<PathBuf>,
}

/// The SDK of the reference IDE's project in `root`: the first module's
/// own, or the project's (`misc.xml`) when the module inherits it or there
/// is no module file. `None` without `.idea` or an SDK.
pub fn project_sdk(root: &Path) -> Option<SdkRef> {
    let idea = root.join(IDEA_DIR);
    if !idea.is_dir() {
        return None;
    }
    let module = module_files(root, &idea)
        .into_iter()
        .find_map(|f| parse_module(&std::fs::read_to_string(f).ok()?).ok());
    if let Some(Module {
        sdk: Some(Some(sdk)),
        ..
    }) = module
    {
        return Some(sdk);
    }
    let text = std::fs::read_to_string(idea.join("misc.xml")).ok()?;
    let mut out = None;
    walk(&text, |path, e| {
        if path.names.len() == 1
            && e.name().as_ref() == "component"
            && attr(e, "name").as_deref() == Some("ProjectRootManager")
            && let (Some(name), Some(type_id)) =
                (attr(e, "project-jdk-name"), attr(e, "project-jdk-type"))
        {
            out = Some(SdkRef { name, type_id });
        }
    })
    .ok()?;
    out
}

/// The SDKs of an IDE's SDK table file (`options/jdk.table.xml`); empty
/// when it cannot be read.
pub fn read_sdk_table(file: &Path) -> Vec<SdkEntry> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let mut out: Vec<SdkEntry> = Vec::new();
    let mut in_table = false;
    let _ = walk(&text, |path, e| {
        let name = e.name();
        if path.names.len() == 1 && name.as_ref() == "component" {
            in_table = attr(e, "name").as_deref() == Some("ProjectJdkTable");
            return;
        }
        if !in_table {
            return;
        }
        if path.names.len() == 2 && name.as_ref() == "jdk" {
            out.push(SdkEntry {
                name: String::new(),
                type_id: String::new(),
                version: None,
                home: None,
            });
            return;
        }
        if path.names.len() != 3 || path.names[2] != "jdk" {
            return;
        }
        let (Some(entry), Some(value)) = (out.last_mut(), attr(e, "value")) else {
            return;
        };
        match name.as_ref() {
            "name" => entry.name = value,
            "type" => entry.type_id = value,
            "version" => entry.version = Some(value),
            "homePath" => entry.home = expand_macros(&value, Path::new(""), Path::new("")),
            _ => {}
        }
    });
    out.retain(|e| !e.name.is_empty());
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Content {
    url: String,
    exclude_folders: Vec<String>,
    exclude_patterns: Vec<String>,
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

/// The names of the elements around the current one, the root first.
struct Ancestors {
    names: Vec<String>,
}

/// Calls `visit` with each start tag of a document and its ancestors.
fn walk(
    text: &str,
    mut visit: impl FnMut(&Ancestors, &BytesStart<'_>),
) -> Result<(), quick_xml::Error> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut ancestors = Ancestors { names: Vec::new() };
    loop {
        match reader.read_event()? {
            Event::Start(e) => {
                visit(&ancestors, &e);
                ancestors.names.push(e.name().as_ref().to_owned());
            }
            Event::Empty(e) => visit(&ancestors, &e),
            Event::End(_) => {
                ancestors.names.pop();
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

/// The module file paths listed in `modules.xml` (macros not expanded).
fn parse_modules(text: &str) -> Result<Vec<String>, quick_xml::Error> {
    let mut out = Vec::new();
    let mut in_manager = false;
    walk(text, |path, e| {
        let name = e.name();
        if path.names.len() == 1 && name.as_ref() == "component" {
            in_manager = attr(e, "name").as_deref() == Some("ProjectModuleManager");
        }
        if in_manager
            && name.as_ref() == "module"
            && path.names.len() == 3
            && path.names[2] == "modules"
            && let Some(file) = attr(e, "filepath").or_else(|| {
                attr(e, "fileurl").and_then(|u| u.strip_prefix("file://").map(str::to_owned))
            })
        {
            out.push(file);
        }
    })?;
    Ok(out)
}

/// The content roots of a module file.
fn parse_module(text: &str) -> Result<Module, quick_xml::Error> {
    let mut out = Module::default();
    let mut in_roots = false;
    walk(text, |path, e| {
        let name = e.name();
        if path.names.len() == 1 && name.as_ref() == "component" {
            in_roots = attr(e, "name").as_deref() == Some("NewModuleRootManager");
            return;
        }
        if !in_roots || path.names.len() < 2 || path.names[1] != "component" {
            return;
        }
        match name.as_ref() {
            "orderEntry" if path.names.len() == 2 && out.sdk.is_none() => {
                match attr(e, "type").as_deref() {
                    Some("jdk") => {
                        if let (Some(name), Some(type_id)) =
                            (attr(e, "jdkName"), attr(e, "jdkType"))
                        {
                            out.sdk = Some(Some(SdkRef { name, type_id }));
                        }
                    }
                    Some("inheritedJdk") => out.sdk = Some(None),
                    _ => {}
                }
            }
            "content" if path.names.len() == 2 => {
                if let Some(url) = attr(e, "url") {
                    out.contents.push(Content {
                        url,
                        ..Content::default()
                    });
                }
            }
            "excludeFolder" if path.names.len() == 3 && path.names[2] == "content" => {
                if let (Some(content), Some(url)) = (out.contents.last_mut(), attr(e, "url")) {
                    content.exclude_folders.push(url);
                }
            }
            "excludePattern" if path.names.len() == 3 && path.names[2] == "content" => {
                if let (Some(content), Some(p)) = (out.contents.last_mut(), attr(e, "pattern")) {
                    content.exclude_patterns.push(p);
                }
            }
            _ => {}
        }
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="ProjectModuleManager">
    <modules>
      <module fileurl="file://$PROJECT_DIR$/.idea/demo.iml" filepath="$PROJECT_DIR$/.idea/demo.iml" />
    </modules>
  </component>
</project>
"#;

    const IML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<module type="PYTHON_MODULE" version="4">
  <component name="NewModuleRootManager">
    <content url="file://$MODULE_DIR$">
      <sourceFolder url="file://$MODULE_DIR$/src" isTestSource="false" />
      <excludeFolder url="file://$MODULE_DIR$/.venv" />
      <excludeFolder url="file://$MODULE_DIR$/docs/_build" />
      <excludePattern pattern="*.egg-info" />
    </content>
    <orderEntry type="jdk" jdkName="Python 3.13 (demo)" jdkType="Python SDK" />
    <orderEntry type="sourceFolder" forTests="false" />
  </component>
  <component name="PyDocumentationSettings">
    <option name="format" value="PLAIN" />
  </component>
</module>
"#;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn paths(e: &Exclusions) -> Vec<&str> {
        e.paths.iter().map(String::as_str).collect()
    }

    #[test]
    fn no_idea_folder_no_import() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_exclusions(dir.path()), None);
    }

    #[test]
    fn reads_module_dir_relative_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".idea/modules.xml", MODULES);
        write(root, ".idea/demo.iml", IML);
        let e = read_exclusions(root).unwrap();
        assert_eq!(paths(&e), [".venv", "docs/_build"]);
        assert_eq!(e.patterns, ["*.egg-info"]);
    }

    #[test]
    fn several_modules_and_content_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            root,
            ".idea/modules.xml",
            r#"<project version="4"><component name="ProjectModuleManager"><modules>
  <module fileurl="file://$PROJECT_DIR$/.idea/main.iml" filepath="$PROJECT_DIR$/.idea/main.iml" />
  <module fileurl="file://$PROJECT_DIR$/backend/backend.iml" filepath="$PROJECT_DIR$/backend/backend.iml" />
  <module fileurl="file://$PROJECT_DIR$/missing.iml" filepath="$PROJECT_DIR$/missing.iml" />
</modules></component></project>"#,
        );
        // Two content roots; `..` leaves the module folder; a folder outside
        // the project is dropped; a jar URL is not a folder.
        write(
            root,
            ".idea/main.iml",
            r#"<module version="4"><component name="NewModuleRootManager">
  <content url="file://$MODULE_DIR$/web">
    <excludeFolder url="file://$MODULE_DIR$/web/node_modules" />
    <excludePattern pattern="*.map" />
  </content>
  <content url="file://$MODULE_DIR$/tools">
    <excludeFolder url="file://$MODULE_DIR$/tools/../build" />
    <excludeFolder url="file://$MODULE_DIR$/../elsewhere" />
    <excludeFolder url="jar://$MODULE_DIR$/lib.jar!/" />
    <excludeFolder url="file://$UNKNOWN_VAR$/x" />
  </content>
</component></module>"#,
        );
        // A module file outside `.idea`: `$MODULE_DIR$` is its own folder.
        write(
            root,
            "backend/backend.iml",
            r#"<module version="4"><component name="NewModuleRootManager">
  <content url="file://$MODULE_DIR$"><excludeFolder url="file://$MODULE_DIR$/venv" /></content>
</component></module>"#,
        );
        let e = read_exclusions(root).unwrap();
        assert_eq!(paths(&e), ["backend/venv", "build", "web/node_modules"]);
        // Patterns of a content root below the project folder are not taken.
        assert!(e.patterns.is_empty());
    }

    #[test]
    fn module_files_without_modules_xml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".idea/demo.iml", IML);
        write(root, ".idea/misc.xml", "<project/>");
        let e = read_exclusions(root).unwrap();
        assert_eq!(paths(&e), [".venv", "docs/_build"]);
    }

    #[test]
    fn broken_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, ".idea/modules.xml", MODULES);
        write(
            root,
            ".idea/demo.iml",
            "<module><component name=\"NewModuleRootManager\"><content",
        );
        assert_eq!(read_exclusions(root), Some(Exclusions::default()));
        // A broken modules.xml falls back to the module files in `.idea`.
        write(root, ".idea/modules.xml", "<project><component");
        write(root, ".idea/demo.iml", IML);
        assert_eq!(
            paths(&read_exclusions(root).unwrap()),
            [".venv", "docs/_build"]
        );
    }

    #[test]
    fn exclusions_only_from_the_module_root_manager() {
        let m = parse_module(
            r#"<module><component name="Other"><content url="file://$MODULE_DIR$"><excludeFolder url="file://$MODULE_DIR$/a" /></content></component>
<component name="NewModuleRootManager"><content url="file://$MODULE_DIR$"><excludeFolder url="file://$MODULE_DIR$/b" /><x><excludeFolder url="file://$MODULE_DIR$/c" /></x></content></component></module>"#,
        )
        .unwrap();
        assert_eq!(m.contents.len(), 1);
        assert_eq!(m.contents[0].exclude_folders, ["file://$MODULE_DIR$/b"]);
    }

    #[test]
    fn the_project_sdk_of_the_module_or_misc_xml() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(project_sdk(root), None);
        write(root, ".idea/modules.xml", MODULES);
        write(root, ".idea/demo.iml", IML);
        let python = |name: &str| SdkRef {
            name: name.into(),
            type_id: "Python SDK".into(),
        };
        assert_eq!(project_sdk(root), Some(python("Python 3.13 (demo)")));
        // The module inherits the project's SDK.
        write(
            root,
            ".idea/demo.iml",
            &IML.replace(
                r#"<orderEntry type="jdk" jdkName="Python 3.13 (demo)" jdkType="Python SDK" />"#,
                r#"<orderEntry type="inheritedJdk" />"#,
            ),
        );
        assert_eq!(project_sdk(root), None);
        write(
            root,
            ".idea/misc.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="Black"><option name="sdkName" value="x" /></component>
  <component name="ProjectRootManager" version="2" project-jdk-name="uv (demo)" project-jdk-type="Python SDK" />
</project>
"#,
        );
        assert_eq!(project_sdk(root), Some(python("uv (demo)")));
    }

    #[test]
    fn reads_the_sdk_table() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("jdk.table.xml");
        assert!(read_sdk_table(&file).is_empty());
        std::fs::write(
            &file,
            r#"<application>
  <component name="ProjectJdkTable">
    <jdk version="2">
      <name value="uv (demo)" />
      <type value="Python SDK" />
      <version value="Python 3.13.14" />
      <homePath value="$USER_HOME$/work/demo/.venv/bin/python" />
      <roots>
        <classPath><root type="composite"><root url="file://$USER_HOME$/x" type="simple" /></root></classPath>
      </roots>
      <additional UV_WORKING_DIR="$USER_HOME$/work/demo" />
    </jdk>
    <jdk version="2">
      <name value="Python 3.12" />
      <type value="Python SDK" />
      <homePath value="/usr/bin/python3.12" />
    </jdk>
    <jdk version="2">
      <name value="remote" />
      <type value="Python SDK" />
      <homePath value="$APPLICATION_HOME_DIR$/python" />
    </jdk>
  </component>
</application>
"#,
        )
        .unwrap();
        let table = read_sdk_table(&file);
        assert_eq!(table.len(), 3);
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .unwrap();
        assert_eq!(
            table[0],
            SdkEntry {
                name: "uv (demo)".into(),
                type_id: "Python SDK".into(),
                version: Some("Python 3.13.14".into()),
                home: Some(home.join("work/demo/.venv/bin/python")),
            }
        );
        assert_eq!(table[1].home, Some(PathBuf::from("/usr/bin/python3.12")));
        assert_eq!(table[1].version, None);
        assert_eq!(table[2].home, None);
    }

    #[test]
    fn expands_macros() {
        let project = Path::new("/w/p");
        let module = Path::new("/w/p/sub");
        let e = |s: &str| expand_macros(s, project, module);
        assert_eq!(
            e("$PROJECT_DIR$/.idea/x.iml"),
            Some(PathBuf::from("/w/p/.idea/x.iml"))
        );
        assert_eq!(e("$MODULE_DIR$"), Some(PathBuf::from("/w/p/sub")));
        assert_eq!(e("$MODULE_DIR$/../a/./b"), Some(PathBuf::from("/w/p/a/b")));
        assert_eq!(e("$MODULE_DIR$x"), None);
        assert_eq!(e("$OTHER$/a"), None);
        assert_eq!(e("/abs/x/../y"), Some(PathBuf::from("/abs/y")));
        assert_eq!(
            module_dir(Path::new("/w/p/.idea/x.iml")),
            PathBuf::from("/w/p")
        );
        assert_eq!(
            module_dir(Path::new("/w/p/sub/x.iml")),
            PathBuf::from("/w/p/sub")
        );
        assert_eq!(
            url_to_path("jar://$MODULE_DIR$/a.jar!/", project, module),
            None
        );
    }
}
