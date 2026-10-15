//! The Shell Script type (the reference's `ShConfigurationType`, a simple
//! type of the Shell Script plugin): a script file or a script text, its
//! options, the working directory, an interpreter and its options, and
//! environment variables (`ShRunConfiguration`). It knows no language, so
//! it lives in the platform.

use std::path::{Path, PathBuf};

use process::CommandLine;

use crate::idea::{IdeaConfiguration, Importer};
use crate::model::{EnvVar, RunConfiguration, TypeInfo};
use crate::{params, paths};

/// The process of a Shell Script configuration in the Run tool window
/// (`ShRunConfigurationProfileState.buildExecutionResult`): the script
/// file through its interpreter (`<interpreter> [options] <script>
/// [script options]`, or the script itself without an interpreter), or
/// the script text with the default shell (`<shell> -c <text>`); the
/// working directory (the project folder when empty); the system
/// environment and the configuration's variables. The reference passes
/// the variables to a script text only in the terminal; Pyoxide passes
/// them in both modes. Running in the terminal comes with the terminal.
pub fn command_line(config: &RunConfiguration, root: &Path) -> Result<CommandLine, String> {
    let text = |k: &str| config.str_option(k).to_owned();
    let args = |k: &str| params::parse(&paths::expand_in_text(config.str_option(k), root));
    let working_dir = paths::resolve(config.str_option(key::WORKING_DIRECTORY), root)
        .unwrap_or_else(|| root.to_owned());
    let mut command = if config.bool_option(key::EXECUTE_SCRIPT_FILE).unwrap_or(true) {
        let script = paths::resolve(config.str_option(key::SCRIPT), root)
            .ok_or_else(|| "Shell script not found".to_owned())?;
        if !script.is_file() {
            return Err("Shell script not found".to_owned());
        }
        match paths::resolve(config.str_option(key::INTERPRETER), root) {
            Some(interpreter) if !interpreter.is_file() => {
                return Err("Interpreter not found".to_owned());
            }
            Some(interpreter) => CommandLine::new(interpreter)
                .args(args(key::INTERPRETER_OPTIONS))
                .arg(script),
            None => CommandLine::new(script),
        }
        .args(args(key::SCRIPT_OPTIONS))
    } else {
        CommandLine::new(default_shell())
            .arg(if cfg!(windows) { "/C" } else { "-c" })
            .arg(text(key::SCRIPT_TEXT))
    };
    if !working_dir.is_dir() {
        return Err("Working directory not found".to_owned());
    }
    command.cwd = Some(working_dir);
    command.inherit_env = true;
    for (name, value) in config.env_values() {
        command = command.with_env(name, value);
    }
    Ok(command)
}

/// The shell a script text runs with: `$SHELL`, else `/bin/sh`
/// (`cmd.exe` on Windows).
fn default_shell() -> PathBuf {
    if cfg!(windows) {
        return std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"));
    }
    std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// Pyoxide's id of the type.
pub const TYPE_ID: &str = "shell-script";
/// The reference's id and display name of the type.
pub const REFERENCE_ID: &str = "ShConfigurationType";
pub const REFERENCE_NAME: &str = "Shell Script";

/// The type.
pub const TYPE: TypeInfo = TypeInfo {
    id: TYPE_ID,
    name: REFERENCE_NAME,
    icon: "fileTypes/shell",
    reference_id: REFERENCE_ID,
    allow_parallel: false,
};

/// Option keys of a Shell Script configuration.
pub mod key {
    /// Run the script file (`EXECUTE_SCRIPT_FILE`, default on); off runs
    /// the script text.
    pub const EXECUTE_SCRIPT_FILE: &str = "execute-script-file";
    pub const SCRIPT: &str = "script";
    pub const SCRIPT_TEXT: &str = "script-text";
    pub const SCRIPT_OPTIONS: &str = "script-options";
    pub const WORKING_DIRECTORY: &str = "working-directory";
    pub const INTERPRETER: &str = "interpreter";
    pub const INTERPRETER_OPTIONS: &str = "interpreter-options";
    /// Run in the terminal (`EXECUTE_IN_TERMINAL`, default on); kept
    /// until Pyoxide has an integrated terminal.
    pub const EXECUTE_IN_TERMINAL: &str = "execute-in-terminal";
}

/// Imports the reference's Shell Script configurations.
pub struct ShellScriptImporter;

impl Importer for ShellScriptImporter {
    fn type_name(&self, reference_type: &str) -> Option<&'static str> {
        (reference_type == REFERENCE_ID).then_some(REFERENCE_NAME)
    }

    fn convert(
        &self,
        idea: &IdeaConfiguration<'_>,
        root: &Path,
        config: &mut RunConfiguration,
    ) -> bool {
        if idea.type_id() != REFERENCE_ID {
            return false;
        }
        config.type_id = TYPE_ID.to_owned();
        let text = |name: &str| idea.field(name).unwrap_or("").to_owned();
        // `INDEPENDENT_<X>` tells whether the path was written with `/`;
        // stored paths always use `/`.
        let path = |name: &str| paths::from_reference(idea.field(name).unwrap_or(""), root);
        config.set_bool(
            key::EXECUTE_SCRIPT_FILE,
            idea.bool_field("EXECUTE_SCRIPT_FILE", true),
        );
        config.set_str(key::SCRIPT, &path("SCRIPT_PATH"));
        config.set_str(key::SCRIPT_TEXT, &text("SCRIPT_TEXT"));
        config.set_str(key::SCRIPT_OPTIONS, &text("SCRIPT_OPTIONS"));
        config.set_str(key::WORKING_DIRECTORY, &path("SCRIPT_WORKING_DIRECTORY"));
        config.set_str(key::INTERPRETER, &path("INTERPRETER_PATH"));
        config.set_str(key::INTERPRETER_OPTIONS, &text("INTERPRETER_OPTIONS"));
        config.set_bool(
            key::EXECUTE_IN_TERMINAL,
            idea.bool_field("EXECUTE_IN_TERMINAL", true),
        );
        // `EnvironmentVariablesData`: `<envs pass-parent-envs="false">`.
        let shared = config.is_shared();
        config.env = idea
            .envs()
            .into_iter()
            .map(|(name, value)| EnvVar::new(name, value, shared))
            .collect();
        config.include_system_env = idea
            .element
            .child("envs")
            .and_then(|e| e.attr("pass-parent-envs"))
            .is_none_or(|v| !v.eq_ignore_ascii_case("false"));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idea::read_workspace;
    use crate::model::Storage;

    #[test]
    fn shell_script_options_are_imported() {
        let text = r#"<project><component name="RunManager">
  <configuration name="build" type="ShConfigurationType">
    <option name="SCRIPT_TEXT" value="" />
    <option name="INDEPENDENT_SCRIPT_PATH" value="true" />
    <option name="SCRIPT_PATH" value="$PROJECT_DIR$/scripts/build.sh" />
    <option name="SCRIPT_OPTIONS" value="--fast &quot;two words&quot;" />
    <option name="INDEPENDENT_SCRIPT_WORKING_DIRECTORY" value="true" />
    <option name="SCRIPT_WORKING_DIRECTORY" value="$PROJECT_DIR$" />
    <option name="INDEPENDENT_INTERPRETER_PATH" value="true" />
    <option name="INTERPRETER_PATH" value="/bin/zsh" />
    <option name="INTERPRETER_OPTIONS" value="-e" />
    <option name="EXECUTE_IN_TERMINAL" value="false" />
    <option name="EXECUTE_SCRIPT_FILE" value="true" />
    <envs pass-parent-envs="false">
      <env name="MODE" value="release" />
    </envs>
    <method v="2" />
  </configuration>
  <configuration name="inline" type="ShConfigurationType">
    <option name="SCRIPT_TEXT" value="echo hi" />
    <option name="EXECUTE_SCRIPT_FILE" value="false" />
    <method v="2" />
  </configuration>
</component></project>"#;
        let ws = read_workspace(text, Path::new("/work/demo"), &[&ShellScriptImporter]).unwrap();
        let build = &ws.configurations[0];
        assert_eq!(build.unique_id, "Shell Script.build");
        let c = &build.config;
        assert_eq!(c.type_id, TYPE_ID);
        assert_eq!(c.storage, Storage::Personal);
        assert_eq!(c.bool_option(key::EXECUTE_SCRIPT_FILE), Some(true));
        assert_eq!(c.str_option(key::SCRIPT), "scripts/build.sh");
        assert_eq!(c.str_option(key::SCRIPT_TEXT), "");
        assert_eq!(c.str_option(key::SCRIPT_OPTIONS), "--fast \"two words\"");
        assert_eq!(c.str_option(key::WORKING_DIRECTORY), ".");
        assert_eq!(c.str_option(key::INTERPRETER), "/bin/zsh");
        assert_eq!(c.str_option(key::INTERPRETER_OPTIONS), "-e");
        assert_eq!(c.bool_option(key::EXECUTE_IN_TERMINAL), Some(false));
        assert!(!c.include_system_env);
        assert_eq!(c.env_values().collect::<Vec<_>>(), [("MODE", "release")]);
        let inline = &ws.configurations[1].config;
        assert_eq!(inline.bool_option(key::EXECUTE_SCRIPT_FILE), Some(false));
        assert_eq!(inline.str_option(key::SCRIPT_TEXT), "echo hi");
        // Defaults of missing fields.
        assert_eq!(inline.bool_option(key::EXECUTE_IN_TERMINAL), Some(true));
        assert!(inline.include_system_env);
    }

    #[cfg(unix)]
    #[test]
    fn command_lines_of_a_file_and_a_text() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("scripts/build.sh"), "echo hi\n").unwrap();
        let mut c = RunConfiguration::new(TYPE_ID, "build", Storage::Personal);
        c.set_str(key::SCRIPT, "scripts/build.sh");
        c.set_str(key::INTERPRETER, "/bin/bash");
        c.set_str(key::INTERPRETER_OPTIONS, "-e");
        c.set_str(key::SCRIPT_OPTIONS, "--root $PROJECT_DIR$ \"two words\"");
        c.set_str(key::WORKING_DIRECTORY, "scripts");
        c.env = vec![EnvVar::new("MODE", "release", false)];
        let cmd = command_line(&c, root).unwrap();
        assert_eq!(cmd.program, PathBuf::from("/bin/bash"));
        let args: Vec<String> = cmd
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-e".to_owned(),
                root.join("scripts/build.sh").display().to_string(),
                "--root".to_owned(),
                root.display().to_string(),
                "two words".to_owned()
            ]
        );
        assert_eq!(cmd.cwd, Some(root.join("scripts")));
        assert!(cmd.inherit_env);
        assert_eq!(cmd.env, [("MODE".into(), Some("release".into()))]);
        // Without an interpreter the script runs itself.
        c.set_str(key::INTERPRETER, "");
        assert_eq!(
            command_line(&c, root).unwrap().program,
            root.join("scripts/build.sh")
        );
        // A script text runs with the shell.
        c.set_bool(key::EXECUTE_SCRIPT_FILE, false);
        c.set_str(key::SCRIPT_TEXT, "echo text");
        let cmd = command_line(&c, root).unwrap();
        assert_eq!(cmd.args, ["-c", "echo text"].map(std::ffi::OsString::from));
        // Errors of the reference's checks.
        c.set_bool(key::EXECUTE_SCRIPT_FILE, true);
        c.set_str(key::SCRIPT, "missing.sh");
        assert_eq!(
            command_line(&c, root).unwrap_err(),
            "Shell script not found"
        );
        c.set_str(key::SCRIPT, "scripts/build.sh");
        c.set_str(key::WORKING_DIRECTORY, "gone");
        assert_eq!(
            command_line(&c, root).unwrap_err(),
            "Working directory not found"
        );
    }
}
