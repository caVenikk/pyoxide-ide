//! `pyoxide-ide`: starts the Pyoxide Python IDE with the folders and files
//! named on the command line.

// A GUI application without a console window of its own on Windows; it
// also lets the Run tool window interrupt a process with Ctrl+C, which
// reaches only processes of the console it is sent to.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let paths = std::env::args_os().skip(1).map(Into::into).collect();
    workbench::run(pyoxide_ide::product(), paths);
}
