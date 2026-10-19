//! What the version control tests share: a repository in the project, waiting
//! for the state of version control, Git for a check's own setup and the diff
//! tabs.

use super::*;

/// Version control of the project, waited for until `f` holds.
pub(super) fn wait_vcs(h: &mut H, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..3000 {
        support::tick(h.cx);
        h.draw();
        let vcs = h.invoke("vcs.state", json!(null));
        if f(&vcs) {
            return vcs;
        }
    }
    panic!("timed out waiting for {what}");
}

/// Makes the project a Git repository with `main`, `feat/x` and `other`
/// (once) and opens the branches popup with its branches.
pub(super) fn open_branches(h: &mut H) -> Value {
    ensure_repository(h);
    h.invoke("vcs.branches-popup", json!(null));
    wait_vcs(h, "the branches", |v| v["branches_popup"]["loaded"] == true)["branches_popup"].clone()
}

/// Makes the project a Git repository (once): `a.py` committed on `main`,
/// branches `feat/x` and `other`, the other files unversioned.
pub(super) fn ensure_repository(h: &mut H) {
    if h.invoke("vcs.state", json!(null))["widget"]["kind"] != "repository" {
        let git = |args: &[&str]| {
            let exe = git::GitExe::find(None).expect("git is installed");
            let out = std::process::Command::new(exe.path)
                .args(args)
                .current_dir(&h.root)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        git(&["add", "a.py"]);
        git(&["commit", "-q", "-m", "first"]);
        git(&["branch", "feat/x"]);
        git(&["branch", "other"]);
        let path = h.root.display().to_string();
        h.invoke("git.init", json!({ "path": path }));
        wait_vcs(h, "the repository", |v| v["widget"]["kind"] == "repository");
    }
}

/// Waits for the active diff to be computed and to satisfy `f`.
pub(super) fn wait_diff(h: &mut H, what: &str, f: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..3000 {
        support::tick(h.cx);
        h.draw();
        let diff = h.state()["diff"].clone();
        if diff["computed"] == json!(true) && f(&diff) {
            return diff;
        }
    }
    panic!(
        "{what}: timed out waiting for the diff: {:#}",
        h.state()["diff"]
    );
}

/// Closes the diff tabs.
pub(super) fn close_diffs(h: &mut H) {
    while let Some(i) = h.state()["tabs"]
        .as_array()
        .and_then(|t| t.iter().position(|t| t.get("view").is_some()))
    {
        h.invoke("tabs.close", json!({ "index": i }));
    }
}

/// Runs git in the project for a check's own setup.
pub(super) fn git_in(h: &H, args: &[&str]) -> String {
    let exe = git::GitExe::find(None).expect("git is installed");
    let out = std::process::Command::new(exe.path)
        .args(args)
        .current_dir(&h.root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The row of a file or group (`changes`, `unversioned`).
pub(super) fn commit_row(commit: &Value, key: &str) -> Value {
    commit["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["path"] == key || r["group"] == key)
        .cloned()
        .unwrap_or(Value::Null)
}
