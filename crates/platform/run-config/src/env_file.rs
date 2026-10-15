//! Environment files ("Paths to .env files"): stored as paths and read at
//! launch. The text is parsed as the reference IDE parses it
//! (`EnvFileParser.parseEnvFile`, see NOTICE): `KEY=value` lines,
//! `export ` before the key, a value in double quotes may span lines (the
//! lines are joined without a break), single quotes around a value are
//! removed, and a `#` after the last quote starts a comment.
//!
//! The reference also runs `.sh` and `.bat` files to read the environment
//! they set; Pyoxide reads every file as text.

use std::path::Path;

/// The variables of an environment file, in order (a later line wins).
pub fn parse(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |key: String, value: String| match out.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => out.push((key, value)),
    };
    let mut current: Option<(String, String)> = None;
    for line in text.lines() {
        match current.take() {
            None => {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                let key = key.trim();
                let key = key.strip_prefix("export ").unwrap_or(key).trim().to_owned();
                let trimmed = value.trim();
                if let Some(quoted) = trimmed.strip_prefix('"') {
                    let value = strip_comment(quoted, '"');
                    match value.strip_suffix('"') {
                        Some(done) => put(key, done.to_owned()),
                        None => current = Some((key, value.to_owned())),
                    }
                } else {
                    let value = strip_comment(value, '\'');
                    let value = value
                        .strip_prefix('\'')
                        .and_then(|v| v.strip_suffix('\''))
                        .unwrap_or(value);
                    put(key, value.to_owned());
                }
            }
            Some((key, mut value)) => {
                let part = strip_comment(line, '"').trim();
                value.push_str(part.strip_suffix('"').unwrap_or(part));
                if part.ends_with('"') {
                    put(key, value);
                } else {
                    current = Some((key, value));
                }
            }
        }
    }
    out
}

/// The text before a `#` that follows the last `delimiter`, trimmed.
fn strip_comment(line: &str, delimiter: char) -> &str {
    match line.rfind('#') {
        Some(pos) if line.rfind(delimiter).is_none_or(|d| pos > d) => line[..pos].trim(),
        _ => line.trim(),
    }
}

/// Reads an environment file. The error is a message for the person
/// ("File not found", "Cannot read file").
pub fn read(path: &Path) -> Result<Vec<(String, String)>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(parse(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("File not found: {}", path.display()))
        }
        Err(_) => Err(format!("Cannot read file: {}", path.display())),
    }
}

/// Splits the reference's `ENV_FILES` value, joined with the path
/// separator of the OS that wrote it (`;` on Windows, `:` elsewhere): a
/// value with `;` is from Windows; otherwise `:` separates, except the
/// colon of a drive letter.
pub fn split_reference_list(value: &str) -> Vec<String> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    if value.contains(';') {
        return value
            .split(';')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
    }
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = value.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b':' {
            continue;
        }
        // `C:/x` or `C:\x`: a drive letter at the start of an entry.
        let drive = i == start + 1
            && bytes[start].is_ascii_alphabetic()
            && matches!(bytes.get(i + 1), Some(b'/') | Some(b'\\'));
        if drive {
            continue;
        }
        out.push(value[start..i].to_owned());
        start = i + 1;
    }
    out.push(value[start..].to_owned());
    out.into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_the_reference() {
        let text = "# comment\nA=1\nexport B = two \nC=\"quoted # not a comment\" # comment\nD='single'\nE=\"multi\n  line\"\nF=x # trailing\nnot a pair\nA=3\nG=\n";
        assert_eq!(
            parse(text),
            [
                ("A", "3"),
                ("B", "two"),
                ("C", "quoted # not a comment"),
                ("D", "single"),
                ("E", "multiline"),
                ("F", "x"),
                ("G", ""),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn env_file_lists_of_both_systems() {
        assert_eq!(
            split_reference_list("$PROJECT_DIR$/.env:$PROJECT_DIR$/.env.local"),
            ["$PROJECT_DIR$/.env", "$PROJECT_DIR$/.env.local"]
        );
        assert_eq!(
            split_reference_list("C:/work/.env;$PROJECT_DIR$/.env.local"),
            ["C:/work/.env", "$PROJECT_DIR$/.env.local"]
        );
        assert_eq!(split_reference_list("C:\\work\\.env"), ["C:\\work\\.env"]);
        assert_eq!(split_reference_list("/a/.env"), ["/a/.env"]);
        assert!(split_reference_list("").is_empty());
    }

    #[test]
    fn missing_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".env");
        assert!(read(&file).unwrap_err().starts_with("File not found"));
        std::fs::write(&file, "K=v\n").unwrap();
        assert_eq!(read(&file).unwrap(), [("K".to_owned(), "v".to_owned())]);
    }
}
