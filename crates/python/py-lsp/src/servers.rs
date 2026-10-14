//! The server specs of the Python tools for `lsp-client`.
//!
//! Routing (ADR 0010): ruff serves diagnostics, code actions, formatting
//! and organize imports; the type server serves everything else, and
//! diagnostics and quick fixes of its own.

use std::path::Path;

use code_intel::{Feature, Features};
use lsp_client::ServerSpec;

use crate::lookup::Executable;
use crate::settings::{Settings, Tool};

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

/// What the specs of a project are made from.
#[derive(Debug, Clone, Copy)]
pub struct SpecContext<'a> {
    /// The project root: the working directory and workspace folder.
    pub root: &'a Path,
    /// The project interpreter (ty's `environment.python`, basedpyright's
    /// `python.pythonPath`).
    pub interpreter: Option<&'a Path>,
    pub settings: &'a Settings,
}

/// The features a tool serves with these settings.
pub fn features(tool: Tool, settings: &Settings) -> Features {
    let _ = settings;
    match tool {
        Tool::Ruff => RUFF_FEATURES,
        Tool::Ty | Tool::Basedpyright => TYPE_SERVER_FEATURES,
    }
}

/// The spec of one tool's server, started with `executable`.
pub fn spec(tool: Tool, executable: &Executable, context: &SpecContext) -> ServerSpec {
    let command = executable
        .command_line(tool.server_args())
        .current_dir(context.root);
    let mut spec = ServerSpec::new(tool.id(), command, context.root)
        .with_languages([PYTHON])
        .with_features(features(tool, context.settings));
    spec.icon = Some(tool.id().to_owned());
    spec
}

/// The specs of a project: ruff when enabled and the type engine's server,
/// for the tools whose executables `find` finds.
pub fn specs(context: &SpecContext, find: impl Fn(Tool) -> Option<Executable>) -> Vec<ServerSpec> {
    context
        .settings
        .servers()
        .into_iter()
        .filter_map(|tool| find(tool).map(|exe| spec(tool, &exe, context)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruff_and_the_type_server() {
        let settings = Settings::default();
        let context = SpecContext {
            root: Path::new("/p"),
            interpreter: None,
            settings: &settings,
        };
        let specs = specs(&context, |tool| {
            Some(Executable::Path(
                format!("/bin/{}", tool.executable()).into(),
            ))
        });
        let ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["ruff", "ty"]);
        assert!(specs[0].features.contains(Feature::Formatting));
        assert!(!specs[1].features.contains(Feature::Formatting));
        assert!(specs[1].features.contains(Feature::Completion));
        assert!(specs[1].serves(PYTHON));
        assert_eq!(specs[1].command.program, Path::new("/bin/ty"));
        assert_eq!(specs[1].command.args, ["server"]);
        assert_eq!(specs[1].command.cwd.as_deref(), Some(Path::new("/p")));
    }
}
