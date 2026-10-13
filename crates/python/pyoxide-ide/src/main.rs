//! `pyoxide-ide`: starts the Pyoxide Python IDE with the folders and files
//! named on the command line.

fn main() {
    let paths = std::env::args_os().skip(1).map(Into::into).collect();
    workbench::run(pyoxide_ide::product(), paths);
}
