//! The Python product's debugger hook ([`DebugAdapterProvider`]): a
//! configuration's run, its interpreter's version check, the helper, the
//! debugpy adapter and its `launch`, and the Python breakpoint types.

use std::sync::RwLock;

use std::path::Path;

use debugger::{
    Adapter, AdapterLaunch, DebugAdapterProvider, DebugRequest, ExceptionBreakpoint,
    ExceptionBreakpointType, ExceptionFilter, ExceptionFilterSetting, IdeaTypes, LaunchKind,
    LineBreakpointType, StepIntoMyCode,
};
use py_run::PythonRun;
use py_run::config::{LaunchContext, PythonOptions};

use crate::breakpoints;
use crate::helper::Helper;
use crate::launch::{self, DebugOptions};

/// The adapter's type at `initialize`.
pub const ADAPTER_ID: &str = "debugpy";

/// What a debug request runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugTarget {
    pub run: PythonRun,
    /// The interpreter's version when the product knows it (`3.13.1`,
    /// `3.14` from `pyvenv.cfg`); otherwise the interpreter is asked.
    pub version: Option<String>,
}

/// The product's part of a debug start: the run of a request with the
/// interpreter it picks for it (the project's, or the configuration's).
/// Blocking, called off the UI thread; an error is a message for the user.
pub trait RunResolver: Send + Sync + 'static {
    fn resolve(&self, request: &DebugRequest) -> Result<DebugTarget, String>;
}

impl<F> RunResolver for F
where
    F: Fn(&DebugRequest) -> Result<DebugTarget, String> + Send + Sync + 'static,
{
    fn resolve(&self, request: &DebugRequest) -> Result<DebugTarget, String> {
        self(request)
    }
}

/// The run of a Python configuration (`py_run::config::launch`) as a debug
/// target with the interpreter of `cx`.
pub fn configuration_target(
    request: &DebugRequest,
    cx: &LaunchContext<'_>,
    version: Option<String>,
) -> Result<DebugTarget, String> {
    let launch = py_run::config::launch(&request.configuration, cx)?;
    Ok(DebugTarget {
        run: launch.run,
        version,
    })
}

/// Debugs Python runs with debugpy.
pub struct PythonDebugProvider {
    resolver: Box<dyn RunResolver>,
    /// A helper given by the product (tests); else it is looked up at each
    /// start, so one fetched while the IDE runs is found.
    helper: Option<Helper>,
    options: RwLock<DebugOptions>,
}

impl PythonDebugProvider {
    pub fn new(resolver: impl RunResolver) -> Self {
        Self {
            resolver: Box::new(resolver),
            helper: None,
            options: RwLock::new(DebugOptions::default()),
        }
    }

    /// Uses this helper instead of looking one up.
    pub fn with_helper(mut self, helper: Helper) -> Self {
        self.helper = Some(helper);
        self
    }

    /// The options of the next sessions.
    pub fn set_options(&self, options: DebugOptions) {
        *self.options.write().unwrap_or_else(|e| e.into_inner()) = options;
    }

    pub fn options(&self) -> DebugOptions {
        self.options
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn helper(&self) -> Result<Helper, String> {
        match &self.helper {
            Some(helper) => Ok(helper.clone()),
            None => Helper::locate().map_err(|missing| missing.to_string()),
        }
    }
}

impl DebugAdapterProvider for PythonDebugProvider {
    /// The run, the interpreter's version (3.8 or later), the helper, then
    /// the adapter with the interpreter and its `launch` arguments.
    fn prepare(&self, request: &DebugRequest) -> Result<AdapterLaunch, String> {
        let target = self.resolver.resolve(request)?;
        let python = target.run.python.clone();
        let version = match target.version.filter(|v| launch::major_minor(v).is_some()) {
            Some(version) => version,
            None => launch::probe_version(&python)?,
        };
        launch::check_version(&python, &version)?;
        let helper = self.helper()?;
        let options = self.options();
        let config = &request.configuration;
        let just_my_code = config.type_id == py_run::config::TYPE_ID
            && PythonOptions::read(config).debug_just_my_code;
        Ok(AdapterLaunch {
            title: config.name.clone(),
            adapter: Adapter::Command(launch::adapter_command(&python, &helper, &options)),
            adapter_id: ADAPTER_ID.to_owned(),
            kind: LaunchKind::Launch,
            arguments: launch::launch_arguments(&target.run, &config.name, just_my_code, &options),
        })
    }

    fn exception_filters(
        &self,
        breakpoints: &[&ExceptionBreakpoint],
        available: &[ExceptionFilter],
    ) -> Vec<ExceptionFilterSetting> {
        breakpoints::filters(breakpoints, available)
    }

    fn default_exception_breakpoints(&self) -> Vec<ExceptionBreakpoint> {
        vec![breakpoints::any_exception()]
    }

    fn idea_types(&self) -> IdeaTypes {
        IdeaTypes {
            line: vec![breakpoints::LINE_BREAKPOINT_TYPE.to_owned()],
            exception: vec![breakpoints::EXCEPTION_BREAKPOINT_TYPE.to_owned()],
            exception_class: breakpoints::property::EXCEPTION.to_owned(),
            configuration_types: vec![(
                py_run::config::REFERENCE_ID.to_owned(),
                py_run::config::TYPE_ID.to_owned(),
            )],
        }
    }

    /// debugpy 1.8.21 cannot switch "just my code" during a session
    /// (pydevd's `setDebuggerProperty` takes no `justMyCode`), so Step
    /// Into My Code is a plain Step Into for now.
    fn step_into_my_code(&self) -> Option<StepIntoMyCode> {
        None
    }

    fn line_breakpoint_type(&self) -> Option<LineBreakpointType> {
        Some(breakpoints::line_type())
    }

    fn stoppable_lines(&self, file: &Path, text: &str) -> Option<Vec<bool>> {
        breakpoints::stoppable_lines(file, text)
    }

    fn exception_breakpoint_type(&self) -> Option<ExceptionBreakpointType> {
        Some(breakpoints::exception_type())
    }

    fn exception_classes(&self) -> Vec<String> {
        breakpoints::BUILTIN_EXCEPTIONS
            .iter()
            .map(|c| (*c).to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use run_config::{RunConfiguration, Storage};
    use serde_json::json;

    use super::*;
    use crate::helper::DEBUGPY_VERSION;

    fn helper(root: &Path) -> Helper {
        let adapter = root.join("debugpy").join("adapter");
        std::fs::create_dir_all(&adapter).unwrap();
        std::fs::write(adapter.join("__main__.py"), "").unwrap();
        let info = root.join(format!("debugpy-{DEBUGPY_VERSION}.dist-info"));
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("RECORD"), "").unwrap();
        Helper::at(root).unwrap()
    }

    fn request(root: &Path, config: RunConfiguration) -> DebugRequest {
        DebugRequest {
            root: root.to_owned(),
            configuration: config,
        }
    }

    fn python_config(root: &Path, script: &str) -> RunConfiguration {
        let mut config = py_run::config::new_configuration("main", Storage::Personal);
        let mut options = PythonOptions::read(&config);
        options.script = script.to_owned();
        options.working_directory = root.to_string_lossy().into_owned();
        options.write(&mut config);
        config
    }

    fn resolver(python: PathBuf, version: Option<&'static str>) -> impl RunResolver {
        move |request: &DebugRequest| {
            let cx = LaunchContext {
                root: &request.root,
                source_roots: &[],
                python: python.clone(),
                uv: None,
            };
            configuration_target(request, &cx, version.map(str::to_owned))
        }
    }

    #[test]
    fn a_python_configuration_is_launched_with_debugpy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let helper = helper(&root.join("helper"));
        let python = root.join("python");
        let provider = PythonDebugProvider::new(resolver(python.clone(), Some("3.13")))
            .with_helper(helper.clone());
        let mut config = python_config(root, "main.py");
        let mut options = PythonOptions::read(&config);
        options.debug_just_my_code = true;
        options.write(&mut config);
        let launch = provider.prepare(&request(root, config)).unwrap();
        assert_eq!(launch.title, "main");
        assert_eq!(launch.adapter_id, "debugpy");
        assert_eq!(launch.kind, LaunchKind::Launch);
        let Adapter::Command(command) = &launch.adapter else {
            panic!("debugpy runs as a process")
        };
        assert_eq!(command.program, python);
        assert_eq!(command.args, vec![helper.adapter().into_os_string()]);
        let args = &launch.arguments;
        assert_eq!(args["python"], json!([python.to_string_lossy()]));
        assert_eq!(
            args["program"],
            json!(root.join("main.py").to_string_lossy())
        );
        assert_eq!(args["cwd"], json!(root.to_string_lossy()));
        assert_eq!(args["console"], "integratedTerminal");
        assert_eq!(args["justMyCode"], true);
        assert_eq!(args["env"]["PYTHONUNBUFFERED"], "1");
        assert_eq!(args["env"]["PYTHONPATH"], json!(root.to_string_lossy()));
    }

    #[test]
    fn options_apply_to_the_next_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let provider = PythonDebugProvider::new(resolver(root.join("python"), Some("3.12.1")))
            .with_helper(helper(&root.join("helper")));
        provider.set_options(DebugOptions {
            show_return_values: true,
            ..DebugOptions::default()
        });
        assert!(provider.options().show_return_values);
        let launch = provider
            .prepare(&request(root, python_config(root, "main.py")))
            .unwrap();
        assert_eq!(launch.arguments["showReturnValue"], true);
        assert_eq!(launch.arguments["justMyCode"], false);
    }

    #[test]
    fn errors_are_messages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // An old interpreter.
        let provider = PythonDebugProvider::new(resolver(root.join("python"), Some("3.7.9")))
            .with_helper(helper(&root.join("helper")));
        let error = provider
            .prepare(&request(root, python_config(root, "main.py")))
            .err()
            .unwrap();
        assert!(error.contains("requires Python 3.8 or later"), "{error}");
        // A configuration without a script.
        let error = provider
            .prepare(&request(root, python_config(root, "")))
            .err()
            .unwrap();
        assert_eq!(error, "Please specify a script name");
        // The resolver's own message.
        let provider =
            PythonDebugProvider::new(|_: &DebugRequest| -> Result<DebugTarget, String> {
                Err("Please select a valid Python interpreter".to_owned())
            });
        let error = provider
            .prepare(&request(root, python_config(root, "main.py")))
            .err()
            .unwrap();
        assert_eq!(error, "Please select a valid Python interpreter");
    }

    #[test]
    fn the_python_breakpoint_types() {
        let provider =
            PythonDebugProvider::new(|_: &DebugRequest| -> Result<DebugTarget, String> {
                Err(String::new())
            });
        let defaults = provider.default_exception_breakpoints();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].class, "BaseException");
        assert!(defaults[0].flag("notifyOnTerminate"));
        let types = provider.idea_types();
        assert_eq!(types.line, ["python-line"]);
        assert_eq!(types.exception, ["python-exception"]);
        assert_eq!(types.exception_class, "exception");
        assert_eq!(
            types.configuration_types,
            [("PythonConfigurationType".to_owned(), "python".to_owned())]
        );
        assert_eq!(provider.step_into_my_code(), None);
        assert_eq!(provider.line_breakpoint_type().unwrap().id, "python-line");
        assert!(
            provider
                .stoppable_lines(Path::new("a.py"), "x = 1\n")
                .is_some()
        );
        assert_eq!(
            provider.exception_breakpoint_type().unwrap().id,
            "python-exception"
        );
        assert!(
            provider
                .exception_classes()
                .contains(&"KeyError".to_owned())
        );
        let any = &defaults[0];
        let filters = provider.exception_filters(
            &[any],
            &[ExceptionFilter {
                id: "uncaught".into(),
                ..ExceptionFilter::default()
            }],
        );
        assert_eq!(filters, [ExceptionFilterSetting::new("uncaught")]);
    }
}
