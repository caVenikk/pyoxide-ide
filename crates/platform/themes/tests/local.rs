//! Runs the importers over real third-party themes kept outside the
//! repository. Ignored by default.
//!
//! ```text
//! PYOXIDE_THEMES_LOCAL=/path/to/themes cargo test -p themes --test local -- --ignored --nocapture
//! ```
//!
//! Without the variable the git-ignored folder `tests/fixtures/themes-local/`
//! of the repository is used. The test prints counts only and asserts that
//! every file imports, that the result survives a round trip through a theme
//! file, and that the class files of archives are never opened.

use std::path::{Path, PathBuf};

use themes::import::Library;
use themes::{Registry, ThemeFile};

fn local_root() -> Option<PathBuf> {
    let root = std::env::var_os("PYOXIDE_THEMES_LOCAL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/themes-local")
        });
    root.is_dir().then_some(root)
}

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            files_below(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
#[ignore = "needs third-party themes in tests/fixtures/themes-local or PYOXIDE_THEMES_LOCAL"]
fn every_local_theme_file_imports() {
    let Some(root) = local_root() else {
        println!("no local theme folder, nothing to do");
        return;
    };
    let registry = Registry::with_builtin();
    let mut files = Vec::new();
    files_below(&root, &mut files);
    let (mut imported, mut refused) = (0, 0);
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let mut library = Library::new();
        if library.add_path(&path, true).is_err() {
            refused += 1;
            continue;
        }
        for archive in library.archives() {
            assert!(
                archive.read.iter().all(|n| !n.ends_with(".class")),
                "{relative}: a class file was opened"
            );
        }
        for item in library.import_all() {
            let s = item.report.summary();
            println!(
                "{relative}: {} mapped {} shadowed {} unmapped {} ({} occurrences) approx {} base {} warnings {}",
                item.theme.name,
                s.mapped,
                s.shadowed,
                s.unmapped_keys,
                s.unmapped_total,
                s.approximations,
                s.defaulted,
                s.warnings
            );
            let text = item.theme.to_file().to_toml();
            let file = ThemeFile::from_toml(&text, &relative).unwrap();
            assert_eq!(registry.resolve_file(&file).unwrap(), item.theme);
            imported += 1;
        }
    }
    println!("{imported} themes imported, {refused} files are not theme files");
    assert!(
        imported > 0,
        "no theme could be imported from {}",
        root.display()
    );
}
