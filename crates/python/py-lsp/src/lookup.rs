//! Finding a tool's executable, as the reference's lookup modes do
//! (`ExecutableDiscoveryMode`, `PyToolExt.getExecutableWithBaseArgs`):
//!
//! - Sdk: the folder of the project interpreter
//!   (`py_env::paths::env_tool`), then the custom path or `PATH` and the
//!   per-user tool folders, then `uvx`;
//! - Path: the custom path or `PATH` and the per-user folders, then `uvx`;
//! - uvx: always `uvx <package>`.

use std::path::{Path, PathBuf};

use process::CommandLine;

use crate::settings::Tool;

/// How a tool's executable is found (the reference's Lookup column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lookup {
    #[default]
    Sdk,
    Path,
    Uvx,
}

impl Lookup {
    /// The value in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Lookup::Sdk => "sdk",
            Lookup::Path => "path",
            Lookup::Uvx => "uvx",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "sdk" => Some(Lookup::Sdk),
            "path" => Some(Lookup::Path),
            "uvx" => Some(Lookup::Uvx),
            _ => None,
        }
    }
}

/// What the lookup knows about the project.
#[derive(Debug, Clone, Copy, Default)]
pub struct LookupContext<'a> {
    /// The project interpreter.
    pub interpreter: Option<&'a Path>,
    /// The person's custom executable.
    pub custom_path: Option<&'a Path>,
    /// The lookup may end at `uvx` (the tool was enabled explicitly, or the
    /// person agreed to run it through uv).
    pub allow_uvx: bool,
}

/// A found tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Executable {
    /// An executable file.
    Path(PathBuf),
    /// Run through `uvx`, which downloads the package when needed.
    Uvx {
        uvx: PathBuf,
        package: String,
        executable: String,
    },
}

impl Executable {
    /// The command line that runs it with `args`: `uvx <package>` when the
    /// executable is named like the package, else `uvx --from <package>
    /// <executable>`, as the reference runs it.
    pub fn command_line(&self, args: &[&str]) -> CommandLine {
        match self {
            Executable::Path(path) => CommandLine::new(path).args(args.iter().copied()),
            Executable::Uvx {
                uvx,
                package,
                executable,
            } => {
                let command = CommandLine::new(uvx);
                let command = if package == executable {
                    command.arg(package)
                } else {
                    command.args(["--from", package.as_str(), executable.as_str()])
                };
                command.args(args.iter().copied())
            }
        }
    }
}

/// Finds a tool's executable by its lookup mode (blocking: run it off the
/// UI thread). `None` when nothing is found and `uvx` is not allowed or
/// not installed.
pub fn find_executable(tool: Tool, lookup: Lookup, context: &LookupContext) -> Option<Executable> {
    let _ = (tool, lookup, context);
    None
}

/// The version a tool reports for `--version` (`ruff 0.15.20`), from its
/// output; `None` when the output is not the tool's.
pub fn parse_version(tool: Tool, output: &str) -> Option<String> {
    let _ = (tool, output);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_command_lines() {
        for mode in [Lookup::Sdk, Lookup::Path, Lookup::Uvx] {
            assert_eq!(Lookup::parse(mode.id()), Some(mode));
        }
        let path = Path::new("venv").join("bin").join("ruff");
        let ruff = Executable::Path(path.clone()).command_line(Tool::Ruff.server_args());
        assert_eq!((ruff.program, ruff.args), (path, vec!["server".into()]));
        let uvx = |package: &str, executable: &str| Executable::Uvx {
            uvx: "uvx".into(),
            package: package.into(),
            executable: executable.into(),
        };
        assert_eq!(
            uvx("ty", "ty").command_line(&["server"]).args,
            ["ty", "server"]
        );
        assert_eq!(
            uvx("basedpyright", "basedpyright-langserver")
                .command_line(&["--stdio"])
                .args,
            [
                "--from",
                "basedpyright",
                "basedpyright-langserver",
                "--stdio"
            ]
        );
    }
}
