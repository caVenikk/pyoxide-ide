//! Where consoles live: one folder per data source under the
//! configuration folder, `consoles/<data source>/console.sql`.

use std::path::{Path, PathBuf};

/// The folder name of a data source: its name with the characters a file
/// name cannot hold replaced.
fn folder_name(source: &str) -> String {
    let name: String = source
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim().trim_matches('.').to_owned();
    if name.is_empty() {
        "_".to_owned()
    } else {
        name
    }
}

/// The file of a data source's console (the first one) under `folder`,
/// the consoles folder of the store, else of the configuration folder.
pub(super) fn console_path(folder: Option<PathBuf>, source: &str) -> PathBuf {
    let folder = folder.unwrap_or_else(|| {
        settings::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("consoles")
    });
    folder.join(folder_name(source)).join("console.sql")
}

/// Creates the file (and its folder) when it is not there.
pub(super) fn ensure(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_source_name_is_a_safe_folder_name() {
        assert_eq!(folder_name("pyoxide-local"), "pyoxide-local");
        assert_eq!(folder_name("a/b:c"), "a_b_c");
        assert_eq!(folder_name(".."), "_");
    }
}
