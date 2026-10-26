//! The one-time import of a reference IDE project's run configurations
//! into Pyoxide's storage, on a whole `.idea` folder written by hand from
//! the reference's sources: per-user and shared configurations, the
//! selection, order, recent list and executors, name clashes, the import
//! markers, `.idea` left byte for byte, personal values only in the
//! personal state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use run_config::import::{import_again, open};
use run_config::shell::{ShellScriptImporter, TYPE as SHELL};
use run_config::{
    ConfigKey, EnvVar, IdeaConfiguration, Importer, RunConfiguration, Storage, Types,
};

/// Converts the test type `TaskType` ("Task"): `COMMAND` and the variables.
struct TaskImporter;

impl Importer for TaskImporter {
    fn type_name(&self, reference_type: &str) -> Option<&'static str> {
        (reference_type == "TaskType").then_some("Task")
    }

    fn convert(
        &self,
        idea: &IdeaConfiguration<'_>,
        _: &Path,
        config: &mut RunConfiguration,
    ) -> bool {
        if idea.type_id() != "TaskType" {
            return false;
        }
        config.type_id = "task".into();
        config.set_str("command", idea.field("COMMAND").unwrap_or(""));
        let shared = config.is_shared();
        config.env = idea
            .envs()
            .into_iter()
            .map(|(n, v)| EnvVar::new(n, v, shared))
            .collect();
        true
    }
}

fn types() -> Types {
    Types::new([
        run_config::TypeInfo {
            id: "task",
            name: "Task",
            icon: "run/run",
            reference_id: "TaskType",
            allow_parallel: false,
        },
        SHELL,
    ])
}

const IMPORTERS: &[&dyn Importer] = &[&TaskImporter, &ShellScriptImporter];

const WORKSPACE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="PropertiesComponent">{
  &quot;keyToString&quot;: {
    &quot;Task.api.executor&quot;: &quot;Debug&quot;,
    &quot;Task.shared.executor&quot;: &quot;Run&quot;
  }
}</component>
  <component name="RunManager" selected="Task.api">
    <configuration name="api" type="TaskType" factoryName="Task" folderName="Servers">
      <option name="COMMAND" value="serve" />
      <envs>
        <env name="SECRET_TOKEN" value="personal-token-value" />
      </envs>
      <method v="2" />
    </configuration>
    <configuration name="shared" type="TaskType" factoryName="Task">
      <option name="COMMAND" value="personal copy" />
      <method v="2" />
    </configuration>
    <configuration name="scratch" type="TaskType" factoryName="Task" temporary="true">
      <option name="COMMAND" value="scratch" />
      <method v="2" />
    </configuration>
    <configuration name="build" type="ShConfigurationType">
      <option name="SCRIPT_PATH" value="$PROJECT_DIR$/build.sh" />
      <option name="EXECUTE_IN_TERMINAL" value="false" />
      <method v="2" />
    </configuration>
    <configuration name="tests in app" type="tests" factoryName="py.test">
      <option name="_new_targetType" value="&quot;PATH&quot;" />
      <envs>
        <env name="TEST_SECRET" value="personal-test-value" />
      </envs>
      <method v="2" />
    </configuration>
    <configuration default="true" type="TaskType" factoryName="Task">
      <option name="COMMAND" value="template" />
      <method v="2" />
    </configuration>
    <list>
      <item itemvalue="Task.shared" />
      <item itemvalue="Task.api" />
      <item itemvalue="Task.scratch" />
      <item itemvalue="Shell Script.build" />
    </list>
    <recent_temporary>
      <list>
        <item itemvalue="Task.scratch" />
      </list>
    </recent_temporary>
  </component>
</project>
"#;

const SHARED: &str = r#"<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="shared" type="TaskType" factoryName="Task">
    <option name="COMMAND" value="team" />
    <envs>
      <env name="PUBLIC_MODE" value="team-value" />
    </envs>
    <method v="2">
      <option name="Make" enabled="false" />
    </method>
  </configuration>
</component>
"#;

const RUN_XML: &str = r#"<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="worker" type="TaskType" factoryName="Task">
    <option name="COMMAND" value="work" />
    <method v="2" />
  </configuration>
  <configuration default="false" name="compose" type="docker-deploy" factoryName="docker-compose.yml">
    <deployment type="docker-compose.yml" />
    <method v="2" />
  </configuration>
</component>
"#;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Every file under a folder with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.clone(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    write(&root, ".idea/workspace.xml", WORKSPACE);
    write(&root, ".idea/runConfigurations/shared.xml", SHARED);
    write(&root, ".run/more.run.xml", RUN_XML);
    (dir, root)
}

fn names(list: &run_config::RunConfigurations) -> Vec<String> {
    list.sorted(&types())
        .iter()
        .map(|c| format!("{}:{}", c.type_id, c.name))
        .collect()
}

#[test]
fn the_reference_configurations_are_imported_once_per_storage_class() {
    let (_dir, root) = project();
    let idea_before = snapshot(&root.join(".idea"));
    let run_before = snapshot(&root.join(".run"));

    let opened = open(&root, IMPORTERS, &types()).unwrap();
    let report = opened.report.clone().unwrap();
    assert_eq!(
        report.message(),
        "Imported 8 run configurations (2 kept read-only). Renamed: 'shared' as 'shared (1)'"
    );
    let list = &opened.list;
    // The reference's order: by type name (no importer here knows the
    // pytest type: unknown, last), folders first, then the configurations
    // the custom order does not name (by name), then the custom order,
    // temporary ones last.
    assert_eq!(
        names(list),
        [
            "idea:docker-deploy:compose",
            "shell-script:build",
            "task:api",
            "task:shared (1)",
            "task:worker",
            "task:shared",
            "task:scratch",
            "idea:tests:tests in app",
        ]
    );
    // The shared one keeps its name; the per-user one is renamed.
    let shared = list.get(&ConfigKey::new("task", "shared")).unwrap();
    assert_eq!(shared.storage, Storage::Shared);
    assert_eq!(shared.str_option("command"), "team");
    assert_eq!(shared.before_launch.len(), 1);
    let renamed = list.get(&ConfigKey::new("task", "shared (1)")).unwrap();
    assert_eq!(renamed.storage, Storage::Personal);
    assert_eq!(renamed.str_option("command"), "personal copy");
    assert!(
        list.get(&ConfigKey::new("task", "scratch"))
            .unwrap()
            .temporary
    );
    assert_eq!(list.selected, Some(ConfigKey::new("task", "api")));
    assert_eq!(list.recent_temporary, [ConfigKey::new("task", "scratch")]);
    assert_eq!(list.executor(&ConfigKey::new("task", "api")), Some("Debug"));
    // "Task.shared" is the shared configuration in the reference.
    assert_eq!(
        list.executor(&ConfigKey::new("task", "shared")),
        Some("Run")
    );
    let tests = list
        .get(&ConfigKey::new("idea:tests", "tests in app"))
        .unwrap();
    assert!(tests.is_unsupported());
    assert_eq!(tests.storage, Storage::Personal);
    let compose = list
        .get(&ConfigKey::new("idea:docker-deploy", "compose"))
        .unwrap();
    assert_eq!(compose.storage, Storage::Shared);
    assert_eq!(
        compose.reference.as_ref().unwrap().type_name.as_deref(),
        Some("Docker")
    );

    // `.idea` and the `*.run.xml` files are byte for byte the same.
    assert_eq!(snapshot(&root.join(".idea")), idea_before);
    assert_eq!(snapshot(&root.join(".run")), run_before);

    // Shared configurations are files in `.app/run/`, the import is
    // marked in the shared settings.
    let oxide = root.join(".app");
    assert!(oxide.join("run/shared.toml").is_file());
    assert!(oxide.join("run/worker.toml").is_file());
    assert!(oxide.join("run/compose.toml").is_file());
    let settings = std::fs::read_to_string(oxide.join("project.toml")).unwrap();
    assert!(
        settings.contains("[run]\nimported-from-idea = true"),
        "{settings}"
    );
    // Personal values are only in the personal state, which Git ignores.
    let committed: Vec<(PathBuf, Vec<u8>)> = snapshot(&oxide)
        .into_iter()
        .filter(|(p, _)| {
            let name = p.file_name().unwrap().to_string_lossy();
            name != "workspace.toml" && name != ".gitignore"
        })
        .collect();
    assert!(!committed.is_empty());
    for (path, bytes) in &committed {
        let text = String::from_utf8_lossy(bytes);
        for secret in ["personal-token-value", "personal-test-value"] {
            assert!(!text.contains(secret), "{} holds {secret}", path.display());
        }
    }
    let personal = std::fs::read_to_string(oxide.join("workspace.toml")).unwrap();
    assert!(personal.contains("personal-token-value"));
    assert_eq!(
        std::fs::read_to_string(oxide.join(".gitignore")).unwrap(),
        "# Default ignored files\n/workspace.toml\n"
    );

    // The next open reads what is stored and imports nothing.
    let again = open(&root, IMPORTERS, &types()).unwrap();
    assert_eq!(again.report, None);
    assert_eq!(names(&again.list), names(list));
    assert_eq!(again.list.selected, list.selected);

    // A deleted configuration does not come back on the next open.
    let mut edited = again.list.clone();
    edited.remove(&ConfigKey::new("task", "api"));
    let shared_file = edited
        .get(&ConfigKey::new("task", "worker"))
        .unwrap()
        .file
        .clone()
        .unwrap();
    edited.remove(&ConfigKey::new("task", "worker"));
    run_config::store::delete_shared(&root, &shared_file).unwrap();
    run_config::store::save(&root, &mut edited).unwrap();
    let reopened = open(&root, IMPORTERS, &types()).unwrap();
    assert_eq!(reopened.report, None);
    assert!(!reopened.list.contains(&ConfigKey::new("task", "api")));
    assert!(!reopened.list.contains(&ConfigKey::new("task", "worker")));
    assert_eq!(snapshot(&root.join(".idea")), idea_before);

    // The import on request adds only what is missing.
    let mut list = reopened.list.clone();
    let report = import_again(&root, &mut list, IMPORTERS).unwrap();
    assert_eq!(report.imported, 2);
    assert!(list.contains(&ConfigKey::new("task", "api")));
    assert!(list.contains(&ConfigKey::new("task", "worker")));
    assert!(!list.contains(&ConfigKey::new("task", "shared (2)")));
    assert_eq!(snapshot(&root.join(".idea")), idea_before);
}

#[test]
fn a_teammate_with_the_shared_files_imports_only_personal_ones() {
    let (_dir, root) = project();
    open(&root, IMPORTERS, &types()).unwrap();
    // The teammate's checkout: the committed files, no personal state.
    std::fs::remove_file(root.join(".app/workspace.toml")).unwrap();
    let opened = open(&root, IMPORTERS, &types()).unwrap();
    let report = opened.report.unwrap();
    // Only `workspace.xml` is imported; the shared ones are there.
    assert_eq!(report.imported, 5);
    assert!(opened.list.contains(&ConfigKey::new("task", "worker")));
    assert!(!opened.list.contains(&ConfigKey::new("task", "worker (1)")));
}

#[test]
fn a_project_without_idea_gets_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let opened = open(dir.path(), IMPORTERS, &types()).unwrap();
    assert!(opened.list.is_empty());
    assert_eq!(opened.report, None);
    assert!(!dir.path().join(".app").exists());
}
