//! The server specs of the Python tools for `lsp-client`.
//!
//! Routing (ADR 0010): ruff serves diagnostics, code actions, formatting
//! and organize imports; the type server serves everything else, and
//! diagnostics and quick fixes of its own. A tool's features follow its
//! settings: no diagnostics with Inspections off, no formatting or
//! organize imports when ruff's Formatting or Import optimizer is off.
//!
//! What each server is told (ADR 0011):
//!
//! - ruff, in `initializationOptions.settings` (ruff reads its editor
//!   settings only there): `showSyntaxErrors = false` while a type server
//!   runs next to it, which reports them itself.
//! - ty: the project interpreter as `environment.python` of its inline
//!   `configuration`, both in `initializationOptions` and as the `ty`
//!   section of `workspace/configuration`.
//! - basedpyright: `{}` as `initializationOptions` (as the reference);
//!   `python.pythonPath` and `basedpyright.analysis` through
//!   `workspace/configuration`: `typeCheckingMode = "standard"` unless the
//!   project sets a mode, and its unused-symbol rules off while ruff runs.
//!
//! `workspace/configuration` is answered by the default of
//! [`lsp_client::ServerCustomizer::configuration`]: the dotted section of
//! the spec's settings.

use std::path::Path;
use std::sync::Arc;

use code_intel::{Feature, Features};
use lsp_client::ServerSpec;
use serde_json::{Value, json};

use crate::lookup::Executable;
use crate::ruff::RuffCustomizer;
use crate::settings::{Settings, Tool};
use crate::type_server::TypeServerCustomizer;

/// The language id of Python documents.
pub const PYTHON: &str = "python";

/// What ruff serves.
pub const RUFF_FEATURES: Features = Features::of(&[
    Feature::Diagnostics,
    Feature::CodeActions,
    Feature::Formatting,
    Feature::OrganizeImports,
]);

/// What the type server serves.
pub const TYPE_SERVER_FEATURES: Features = Features::all()
    .without(Feature::Formatting)
    .without(Feature::OrganizeImports);

/// basedpyright's rules about unused code that ruff reports too
/// (`F401`, `F841`, ...): off while ruff runs.
pub const BASEDPYRIGHT_UNUSED_RULES: [&str; 5] = [
    "reportUnusedImport",
    "reportUnusedVariable",
    "reportUnusedClass",
    "reportUnusedFunction",
    "reportUnusedParameter",
];

/// What the specs of a project are made from.
#[derive(Debug, Clone, Copy)]
pub struct SpecContext<'a> {
    /// The project root: the working directory and workspace folder.
    pub root: &'a Path,
    /// The project interpreter (ty's `environment.python`, basedpyright's
    /// `python.pythonPath`).
    pub interpreter: Option<&'a Path>,
    pub settings: &'a Settings,
    /// Ruff's customizer (rule names, its added actions); none when
    /// `None`.
    pub ruff: Option<&'a Arc<RuffCustomizer>>,
}

/// The features a tool serves with these settings.
pub fn features(tool: Tool, settings: &Settings) -> Features {
    let own = settings.tool(tool);
    let mut features = match tool {
        Tool::Ruff => RUFF_FEATURES,
        Tool::Ty | Tool::Basedpyright => TYPE_SERVER_FEATURES,
    };
    if !own.inspections {
        features = features.without(Feature::Diagnostics);
    }
    if tool == Tool::Ruff {
        if !own.formatting {
            features = features.without(Feature::Formatting);
        }
        if !own.import_optimizer {
            features = features.without(Feature::OrganizeImports);
        }
    }
    features
}

/// The spec of one tool's server, started with `executable`; `peers` are
/// the tools whose servers run next to it.
pub fn spec(
    tool: Tool,
    executable: &Executable,
    context: &SpecContext,
    peers: &[Tool],
) -> ServerSpec {
    let command = executable
        .command_line(tool.server_args())
        .current_dir(context.root);
    let mut spec = ServerSpec::new(tool.id(), command, context.root)
        .with_languages([PYTHON])
        .with_features(features(tool, context.settings))
        .with_settings(workspace_settings(tool, context, peers));
    spec.name = tool.name().to_owned();
    spec.icon = Some(tool.id().to_owned());
    spec.initialization_options = initialization_options(tool, context, peers);
    if tool == Tool::Ruff
        && let Some(ruff) = context.ruff
    {
        spec = spec.with_customizer(ruff.clone());
    }
    if tool.is_type_server() {
        spec = spec.with_customizer(Arc::new(TypeServerCustomizer::new(tool)));
    }
    if tool == Tool::Basedpyright {
        // The pyright family asks for its settings once told they changed
        // (the reference's `PyrightEmptyConfigurationSender`).
        spec = spec.with_empty_configuration_change();
    }
    spec
}

/// The specs of a project: ruff when enabled and the type engine's server,
/// for the tools whose executables `find` finds.
pub fn specs(context: &SpecContext, find: impl Fn(Tool) -> Option<Executable>) -> Vec<ServerSpec> {
    let found: Vec<(Tool, Executable)> = context
        .settings
        .servers()
        .into_iter()
        .filter_map(|tool| find(tool).map(|exe| (tool, exe)))
        .collect();
    let running: Vec<Tool> = found.iter().map(|(tool, _)| *tool).collect();
    found
        .iter()
        .map(|(tool, exe)| {
            let peers: Vec<Tool> = running.iter().copied().filter(|t| t != tool).collect();
            spec(*tool, exe, context, &peers)
        })
        .collect()
}

/// `initializationOptions` of a tool's server.
pub fn initialization_options(tool: Tool, context: &SpecContext, peers: &[Tool]) -> Option<Value> {
    match tool {
        Tool::Ruff => {
            let type_server = peers.iter().any(|t| t.is_type_server());
            Some(json!({ "settings": { "showSyntaxErrors": !type_server } }))
        }
        Tool::Ty => ty_options(context),
        Tool::Basedpyright => Some(json!({})),
    }
}

/// The settings `workspace/configuration` answers from, by section.
pub fn workspace_settings(tool: Tool, context: &SpecContext, peers: &[Tool]) -> Value {
    match tool {
        Tool::Ruff => Value::Null,
        Tool::Ty => json!({ "ty": ty_options(context).unwrap_or_else(|| json!({})) }),
        Tool::Basedpyright => {
            let mut analysis = serde_json::Map::new();
            if !type_checking_mode_configured(context.root) {
                analysis.insert("typeCheckingMode".into(), json!("standard"));
            }
            if peers.contains(&Tool::Ruff) {
                let off: serde_json::Map<String, Value> = BASEDPYRIGHT_UNUSED_RULES
                    .iter()
                    .map(|rule| ((*rule).to_owned(), json!("none")))
                    .collect();
                analysis.insert("diagnosticSeverityOverrides".into(), Value::Object(off));
            }
            let mut python = serde_json::Map::new();
            if let Some(interpreter) = context.interpreter {
                python.insert("pythonPath".into(), json!(interpreter.to_string_lossy()));
            }
            json!({
                "python": python,
                "basedpyright": { "analysis": analysis },
            })
        }
    }
}

/// ty's client options: the interpreter as `environment.python` of the
/// inline configuration; `None` without an interpreter (ty then finds
/// the environment itself).
fn ty_options(context: &SpecContext) -> Option<Value> {
    let interpreter = context.interpreter?;
    Some(json!({
        "configuration": { "environment": { "python": interpreter.to_string_lossy() } },
    }))
}

/// The project sets basedpyright's (or pyright's) `typeCheckingMode`
/// itself: in `pyrightconfig.json`, or in `[tool.basedpyright]` or
/// `[tool.pyright]` of `pyproject.toml`. Blocking: reads files.
pub fn type_checking_mode_configured(root: &Path) -> bool {
    const KEY: &str = "typeCheckingMode";
    if std::fs::read_to_string(root.join("pyrightconfig.json"))
        .is_ok_and(|text| text.contains(&format!("\"{KEY}\"")))
    {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(root.join("pyproject.toml")) else {
        return false;
    };
    let Ok(pyproject) = text.parse::<toml::Table>() else {
        return false;
    };
    ["basedpyright", "pyright"].iter().any(|tool| {
        pyproject
            .get("tool")
            .and_then(|t| t.get(*tool))
            .and_then(|t| t.get(KEY))
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use lsp_client::{NoCustomization, ServerCustomizer};

    use super::*;
    use crate::settings::TypeEngine;

    fn found(tool: Tool) -> Option<Executable> {
        Some(Executable::Path(
            format!("/bin/{}", tool.executable()).into(),
        ))
    }

    fn answer(spec: &ServerSpec, section: &str) -> Value {
        NoCustomization.configuration(Some(section), None, &spec.settings)
    }

    #[test]
    fn ruff_and_the_type_server() {
        let settings = Settings::default();
        let ruff = Arc::new(RuffCustomizer::new("/p"));
        let interpreter = PathBuf::from("/p/.venv/bin/python");
        let context = SpecContext {
            root: Path::new("/p"),
            interpreter: Some(&interpreter),
            settings: &settings,
            ruff: Some(&ruff),
        };
        let specs = specs(&context, found);
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ruff", "ty"]);
        let (r, ty) = (&specs[0], &specs[1]);
        assert_eq!(r.name, "Ruff");
        assert_eq!(r.icon.as_deref(), Some("ruff"));
        assert!(r.customizer.is_some() && ty.customizer.is_some());
        assert!(r.features.contains(Feature::Formatting));
        assert!(!r.features.contains(Feature::Completion));
        assert!(!ty.features.contains(Feature::Formatting));
        assert!(ty.features.contains(Feature::Completion));
        assert!(ty.features.contains(Feature::Diagnostics));
        assert!(ty.serves(PYTHON));
        assert_eq!(ty.command.program, Path::new("/bin/ty"));
        assert_eq!(ty.command.args, ["server"]);
        assert_eq!(ty.command.cwd.as_deref(), Some(Path::new("/p")));
        assert_eq!(ty.root, Path::new("/p"));
        assert_eq!(r.command.args, ["server"]);

        // ty reports syntax errors; ruff does not while ty runs.
        assert_eq!(
            r.initialization_options,
            Some(json!({ "settings": { "showSyntaxErrors": false } }))
        );
        let ty_options = json!({
            "configuration": { "environment": { "python": "/p/.venv/bin/python" } }
        });
        assert_eq!(ty.initialization_options.as_ref(), Some(&ty_options));
        assert_eq!(answer(ty, "ty"), ty_options);
        assert_eq!(answer(r, "ruff"), Value::Null);
    }

    #[test]
    fn without_a_type_server_ruff_keeps_syntax_errors() {
        let settings = Settings::default();
        let context = SpecContext {
            root: Path::new("/p"),
            interpreter: None,
            settings: &settings,
            ruff: None,
        };
        let specs = specs(&context, |tool| {
            (tool == Tool::Ruff).then(|| found(tool)).flatten()
        });
        assert_eq!(specs.len(), 1);
        assert!(specs[0].customizer.is_none());
        assert_eq!(
            specs[0].initialization_options,
            Some(json!({ "settings": { "showSyntaxErrors": true } }))
        );

        // ty without an interpreter: it finds the environment itself.
        let ty = spec(Tool::Ty, &found(Tool::Ty).unwrap(), &context, &[Tool::Ruff]);
        assert_eq!(ty.initialization_options, None);
        assert_eq!(answer(&ty, "ty"), json!({}));
    }

    #[test]
    fn basedpyright() {
        let mut settings = Settings::default();
        settings.set_type_engine(TypeEngine::Basedpyright);
        let dir = tempfile::tempdir().unwrap();
        let interpreter = dir.path().join(".venv").join("bin").join("python");
        let context = SpecContext {
            root: dir.path(),
            interpreter: Some(&interpreter),
            settings: &settings,
            ruff: None,
        };
        let specs = specs(&context, found);
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ruff", "basedpyright"]);
        let bp = &specs[1];
        assert_eq!(bp.name, "Basedpyright");
        assert_eq!(
            bp.command.program,
            Path::new("/bin/basedpyright-langserver")
        );
        assert_eq!(bp.command.args, ["--stdio"]);
        assert_eq!(bp.initialization_options, Some(json!({})));
        assert!(bp.empty_configuration_change);
        assert!(!specs[0].empty_configuration_change);
        assert_eq!(
            answer(bp, "python"),
            json!({ "pythonPath": interpreter.to_string_lossy() })
        );
        let analysis = answer(bp, "basedpyright.analysis");
        assert_eq!(analysis["typeCheckingMode"], "standard");
        for rule in BASEDPYRIGHT_UNUSED_RULES {
            assert_eq!(analysis["diagnosticSeverityOverrides"][rule], "none");
        }
        assert_eq!(
            answer(bp, "basedpyright.analysis.typeCheckingMode"),
            "standard"
        );

        // Without ruff the unused rules stay; a mode the project sets wins.
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"demo\"\n\n[tool.basedpyright]\ntypeCheckingMode = \"strict\"\n",
        )
        .unwrap();
        let alone = spec(
            Tool::Basedpyright,
            &found(Tool::Basedpyright).unwrap(),
            &context,
            &[],
        );
        assert_eq!(answer(&alone, "basedpyright.analysis"), json!({}));
        assert_eq!(
            initialization_options(Tool::Ruff, &context, &[]),
            Some(json!({ "settings": { "showSyntaxErrors": true } }))
        );
    }

    #[test]
    fn project_type_checking_modes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!type_checking_mode_configured(root));
        std::fs::write(
            root.join("pyproject.toml"),
            "[tool.ruff]\nline-length = 100\n",
        )
        .unwrap();
        assert!(!type_checking_mode_configured(root));
        std::fs::write(
            root.join("pyproject.toml"),
            "[tool.pyright]\ntypeCheckingMode = \"basic\"\n",
        )
        .unwrap();
        assert!(type_checking_mode_configured(root));
        std::fs::write(root.join("pyproject.toml"), "not toml [").unwrap();
        assert!(!type_checking_mode_configured(root));
        std::fs::write(
            root.join("pyrightconfig.json"),
            "{\n  // comments are allowed\n  \"typeCheckingMode\": \"off\"\n}\n",
        )
        .unwrap();
        assert!(type_checking_mode_configured(root));
    }

    #[test]
    fn features_follow_the_settings() {
        let mut settings = Settings::default();
        settings.ruff.formatting = false;
        settings.ruff.import_optimizer = false;
        settings.ty.inspections = false;
        let ruff = features(Tool::Ruff, &settings);
        assert!(ruff.contains(Feature::Diagnostics) && ruff.contains(Feature::CodeActions));
        assert!(!ruff.contains(Feature::Formatting));
        assert!(!ruff.contains(Feature::OrganizeImports));
        let ty = features(Tool::Ty, &settings);
        assert!(!ty.contains(Feature::Diagnostics));
        assert!(ty.contains(Feature::Completion) && ty.contains(Feature::CodeActions));
        settings.ruff.inspections = false;
        assert!(!features(Tool::Ruff, &settings).contains(Feature::Diagnostics));

        // A disabled ruff and a type server turned off: no specs.
        settings.ruff.enabled = false;
        settings.ty.enabled = false;
        let context = SpecContext {
            root: Path::new("/p"),
            interpreter: None,
            settings: &settings,
            ruff: None,
        };
        assert!(specs(&context, found).is_empty());
    }

    #[test]
    fn through_uvx() {
        let settings = Settings::default();
        let context = SpecContext {
            root: Path::new("/p"),
            interpreter: None,
            settings: &settings,
            ruff: None,
        };
        let specs = specs(&context, |tool| {
            Some(Executable::uvx(
                tool,
                PathBuf::from("/home/u/.local/bin/uvx"),
            ))
        });
        assert_eq!(specs[0].command.args, ["ruff", "server"]);
        assert_eq!(specs[1].command.args, ["ty", "server"]);
        assert_eq!(specs[1].command.cwd.as_deref(), Some(Path::new("/p")));
    }
}
