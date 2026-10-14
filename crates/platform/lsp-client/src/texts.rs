//! Texts of files that are not open, for converting a server's positions
//! in them to the model's char-based points (a declaration in another
//! file, an edit of a file the user has not opened). Read off the UI
//! thread, bounded in size.

use std::fs;
use std::path::Path;

use text::Rope;

use crate::encoding::Text;

/// The largest file read to convert positions.
pub(crate) const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

/// The text of a file on disk, decoded as the editor decodes it (BOM,
/// UTF-16, line breaks normalised to `\n`).
pub(crate) fn read(path: &Path) -> Result<Text, String> {
    let shown = path.display();
    let metadata = fs::metadata(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    if metadata.len() > MAX_FILE_SIZE {
        return Err(format!("{shown} is too large"));
    }
    let bytes = fs::read(path).map_err(|e| format!("cannot read {shown}: {e}"))?;
    let decoded = text::decode(&bytes).map_err(|_| format!("{shown} is not a text file"))?;
    Ok(Text::new(Rope::from_str(&decoded.text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_read_as_the_editor_reads_them() {
        let dir = std::env::temp_dir().join(format!("lsp-client-texts-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("crlf.py");
        fs::write(&file, b"\xEF\xBB\xBFa = 1\r\nb = 2\r\n").unwrap();
        let text = read(&file).unwrap();
        assert_eq!(text.rope().to_string(), "a = 1\nb = 2\n");
        let binary = dir.join("x.bin");
        fs::write(&binary, b"\0\0\0").unwrap();
        assert!(read(&binary).unwrap_err().contains("not a text file"));
        assert!(read(&dir.join("missing.py")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
