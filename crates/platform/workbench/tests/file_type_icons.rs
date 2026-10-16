//! File icons in the project tree (tests/scenarios/editor/file-type-icons.md):
//! by whole name, pattern and extension in the reference's order, a
//! product's own associations, icons of some YAML files, and files no
//! association names drawn by their content (text, binary, `#!` scripts),
//! read when the folder is listed.

use std::path::Path;
use std::rc::Rc;

use gpui::{AppContext as _, TestAppContext, WindowHandle};
use serde_json::{Value, json};
use ui::icons::IconName;
use workbench::{FileAssociation, FileType, Product, Workbench};

fn open(cx: &mut TestAppContext, root: &Path) -> WindowHandle<Workbench> {
    let product = Product {
        name: "Test IDE",
        file_types: vec![FileType {
            hash_bangs: &["python"],
            ..FileType::plain("Python", &["py"], IconName("python/file"))
        }],
        file_associations: vec![FileAssociation {
            mask: "uv.lock",
            file_type: "TOML",
        }],
        ..Product::default()
    };
    let window = cx.update(|cx| {
        workbench::init(cx, &product);
        workbench::open_main_window(cx, Rc::new(product))
    });
    cx.run_until_parked();
    invoke(
        cx,
        window,
        "project.open-folder",
        json!({"path": root.display().to_string()}),
    );
    window
}

fn invoke(
    cx: &mut TestAppContext,
    window: WindowHandle<Workbench>,
    id: &str,
    args: Value,
) -> Value {
    let out = cx
        .update_window(window.into(), |_, window, cx| {
            ui::command::invoke(id, args, window, cx)
        })
        .unwrap()
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    cx.run_until_parked();
    out
}

fn icons(cx: &mut TestAppContext, window: WindowHandle<Workbench>) -> Vec<(String, String)> {
    let state = invoke(cx, window, "workbench.state", json!(null));
    assert_eq!(
        state["busy"],
        json!(false),
        "background work did not finish"
    );
    state["tree"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|r| {
            let path = r["path"].as_str().unwrap().to_owned();
            (path, r["icon"].as_str().unwrap().to_owned())
        })
        .collect()
}

/// A project root as the reference shows it: Docker, GitLab CI and
/// pre-commit files get their own icons, `.env` files, `.python-version`
/// and `Dockerfile.dev` the text icon (the last two by their content).
#[gpui::test]
fn project_root_files_get_the_reference_icons(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let files: &[(&str, &[u8])] = &[
        (".dockerignore", b".venv\n"),
        (".env", b"A=1\n"),
        (".env.example", b"A=\n"),
        (".gitignore", b"*.pyc\n"),
        (".gitlab-ci.yml", b"stages: []\n"),
        (".pre-commit-config.yaml", b"repos: []\n"),
        (".python-version", b"3.13\n"),
        ("alembic.ini", b"[alembic]\n"),
        ("data.bin", b"\x00\x01"),
        ("docker-compose.dev.yml", b"services: {}\n"),
        ("Dockerfile", b"FROM python\n"),
        ("Dockerfile.dev", b"FROM python\n"),
        ("LICENSE", b"MIT\n"),
        ("logs.log", b"started\n"),
        ("manage", b"#!/usr/bin/env python3\n"),
        ("pyproject.toml", b"[project]\n"),
        ("README.md", b"# x\n"),
        ("run.py", b"print()\n"),
        ("uv.lock", b"version = 1\n"),
    ];
    for (name, bytes) in files {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let window = open(cx, root);
    let icons = icons(cx, window);
    let expected = [
        (".dockerignore", "fileTypes/dockerIgnore"),
        (".env", "fileTypes/text"),
        (".env.example", "fileTypes/text"),
        (".gitignore", "fileTypes/ignored"),
        (".gitlab-ci.yml", "fileTypes/gitLabLogo"),
        (".pre-commit-config.yaml", "fileTypes/preCommit"),
        (".python-version", "fileTypes/text"),
        ("alembic.ini", "fileTypes/ini"),
        ("data.bin", "fileTypes/unknown"),
        ("docker-compose.dev.yml", "fileTypes/dockerCompose"),
        ("Dockerfile", "fileTypes/docker"),
        ("Dockerfile.dev", "fileTypes/text"),
        ("LICENSE", "fileTypes/text"),
        ("logs.log", "fileTypes/text"),
        ("manage", "python/file"),
        ("pyproject.toml", "fileTypes/toml"),
        ("README.md", "fileTypes/markdown"),
        ("run.py", "python/file"),
        ("uv.lock", "fileTypes/toml"),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(p, i)| (p.to_string(), i.to_string()))
        .collect();
    assert_eq!(icons, expected);
}

/// A file whose content changes is read again when its folder is listed
/// again (Reload All from Disk).
#[gpui::test]
fn content_is_read_again_when_the_folder_is_listed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("blob"), b"\x00\x01").unwrap();
    let window = open(cx, root);
    assert_eq!(
        icons(cx, window),
        [("blob".into(), "fileTypes/unknown".into())]
    );
    std::fs::write(root.join("blob"), b"now text\n").unwrap();
    invoke(cx, window, "workbench.synchronize", json!(null));
    assert_eq!(
        icons(cx, window),
        [("blob".into(), "fileTypes/text".into())]
    );
}
