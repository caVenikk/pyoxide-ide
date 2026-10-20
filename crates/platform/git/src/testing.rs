//! Temporary repositories for tests of this crate and of the crates built on
//! it. Git for Windows defaults to `core.autocrlf=true`, which writes CRLF
//! into the files of a checkout (and into what `git add` stores), so a test
//! that compares bytes fails on Windows only. Every test repository is made
//! here: the settings are pinned before the first file is written, in the
//! repository's own config.

use std::path::Path;

use crate::GitExe;

/// The settings pinned in every test repository.
pub const PINNED: [(&str, &str); 2] = [("core.autocrlf", "false"), ("core.eol", "lf")];

/// A new repository on `main` in `dir` (an existing folder), with
/// [`PINNED`] in its own config.
pub fn init(dir: &Path) {
    run(dir, &["init", "-q", "-b", "main"]);
    for (key, value) in PINNED {
        run(dir, &["config", key, value]);
    }
}

/// `git clone` of `source` (a path or URL) into the folder `name` of `cwd`
/// with [`PINNED`] in force from the checkout on, kept in the clone's config.
pub fn clone(cwd: &Path, source: &str, name: &str) {
    let mut args = vec!["clone".to_owned(), "-q".to_owned()];
    for (key, value) in PINNED {
        args.extend(["-c".to_owned(), format!("{key}={value}")]);
    }
    args.extend([source.to_owned(), name.to_owned()]);
    run(cwd, &args);
}

/// Runs git in `dir`; panics on failure.
fn run<S: AsRef<std::ffi::OsStr> + std::fmt::Debug>(dir: &Path, args: &[S]) {
    let exe = GitExe::find(None).expect("git is installed");
    let out = exe
        .command()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
