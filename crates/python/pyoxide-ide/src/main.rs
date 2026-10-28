//! `pyoxide-ide`: starts the Pyoxide Python IDE with the folders and files
//! named on the command line.

// A GUI application without a console window of its own on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The platform learns the product's names before anything reads a
    // setting.
    brand::install();
    // On Windows the IDE is also its own Ctrl+C sender for the Run tool
    // window (the reference uses a helper program the same way): started
    // with `--<slug>-send-ctrl-c <pid>` it only sends Ctrl+C and exits.
    process::run_interrupt_helper_if_requested();
    process::set_interrupt_helper(process::InterruptHelper::current_exe());
    // The configured Python interpreters live in the configuration folder.
    pyoxide_ide::interpreter::keep_in_config_dir();
    let paths = std::env::args_os().skip(1).map(Into::into).collect();
    workbench::run(pyoxide_ide::product(), paths);
}
