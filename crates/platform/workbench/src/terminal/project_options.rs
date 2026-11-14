//! Personal options of a local terminal. Reads and writes share a per-project
//! queue so a session opened immediately after Apply sees the chosen options.
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use futures::{FutureExt as _, future::Shared};
use gpui::{App, AppContext as _, Global, Task};
use project::workspace;
use run_config::EnvVar;
use settings_import::pycharm::options::project_terminal;

const TABLE: &str = "terminal.settings";

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Options {
    pub requires_isolated_env: bool,
    pub directory: String,
    pub env: Vec<EnvVar>,
    pub toggle: Option<bool>,
}

impl Options {
    pub fn apply_to(self, request: &mut terminal::StartRequest, root: &Path) {
        request.requires_isolated_env |= self.requires_isolated_env;
        if !self.directory.is_empty() && request.start_directory.is_none() {
            let directory = self
                .directory
                .replace("$PROJECT_DIR$", &root.to_string_lossy());
            request.start_directory = Some(root.join(directory));
        }
        request.project_env.splice(
            0..0,
            self.env
                .into_iter()
                .filter_map(|v| Some((v.name, v.value?))),
        );
    }

    fn load(root: &Path) -> Result<Self, String> {
        if let Some(table) = workspace::load_table(root, TABLE).map_err(|e| e.to_string())? {
            let directory = table
                .get("start-directory")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let env = table
                .get("environment")
                .and_then(|v| v.as_array())
                .map(|vars| {
                    vars.iter()
                        .filter_map(|v| {
                            let (name, value) = v.as_str()?.split_once('=')?;
                            Some(EnvVar::new(name, value, false))
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Ok(Self {
                directory,
                env,
                toggle: None,
                requires_isolated_env: false,
            });
        }
        let path = root.join(".idea/workspace.xml");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let Some(imported) =
            project_terminal(&text).map_err(|e| format!("{}: {e}", path.display()))?
        else {
            return Ok(Self::default());
        };
        if imported.requires_isolated_env {
            return Ok(Self {
                requires_isolated_env: true,
                ..Self::default()
            });
        }
        let options = Self {
            requires_isolated_env: false,
            toggle: None,
            directory: imported.directory,
            env: imported
                .env
                .into_iter()
                .map(|(k, v)| EnvVar::new(k, v, false))
                .collect(),
        };
        options.save(root)?;
        Ok(options)
    }

    fn save(&self, root: &Path) -> Result<(), String> {
        // An empty directory also marks a cleared import; it must stay cleared.
        let env = self
            .env
            .iter()
            .filter_map(|v| Some(format!("{}={}", v.name, v.value.as_ref()?)))
            .collect();
        workspace::save_toml_values(
            root,
            TABLE,
            vec![
                ("start-directory", Some(self.directory.clone().into())),
                ("environment", Some(env)),
            ],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

type Pending = Shared<Task<Result<Options, String>>>;

#[derive(Default)]
struct Queue(RefCell<HashMap<PathBuf, Pending>>);
impl Global for Queue {}

pub(super) fn init(cx: &mut App) {
    cx.set_global(Queue::default());
    cx.on_app_quit(|cx| {
        let tasks: Vec<_> = cx
            .global::<Queue>()
            .0
            .borrow_mut()
            .drain()
            .map(|(_, t)| t)
            .collect();
        async move {
            futures::future::join_all(tasks).await;
        }
    })
    .detach();
}

/// Share an in-flight read/import or a pending write with the page and planner.
pub(crate) fn read(root: &Path, cx: &App) -> Pending {
    let queue = cx.global::<Queue>();
    queue.0.borrow_mut().retain(|_, t| t.peek().is_none());
    if let Some(task) = queue.0.borrow().get(root) {
        return task.clone();
    }
    let toggle = super::project_toggle(cx);
    let path = root.to_owned();
    let task = cx
        .background_spawn(async move {
            let mut options = Options::load(&path)?;
            if !options.requires_isolated_env {
                options.toggle = toggle.map(|t| (t.load)(&path));
            }
            Ok(options)
        })
        .shared();
    queue.0.borrow_mut().insert(root.to_owned(), task.clone());
    task
}

pub(crate) fn save(root: PathBuf, before: Options, options: Options, cx: &App) -> Pending {
    let queue = cx.global::<Queue>();
    let toggle = super::project_toggle(cx);
    let previous = queue.0.borrow_mut().remove(&root);
    let path = root.clone();
    let task = cx
        .background_spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            if options.directory != before.directory || options.env != before.env {
                options.save(&path)?;
            }
            if options.toggle != before.toggle
                && let (Some(value), Some(t)) = (options.toggle, toggle)
            {
                (t.save)(&path, value)?;
            }
            Ok(options)
        })
        .shared();
    queue.0.borrow_mut().insert(root, task.clone());
    task
}
