//! File types: the icon of a file and the editor settings for it. The
//! platform knows common text formats; a product adds its languages.

use std::path::Path;
use std::sync::Arc;

use editor::{EditorConfig, Highlighter, WrapRule};
use ui::icons::IconName;

/// Creates the highlighter of a file type for a new editor.
pub type HighlighterFactory = Arc<dyn Fn() -> Box<dyn Highlighter> + Send + Sync>;

/// A kind of file the product knows.
#[derive(Clone)]
pub struct FileType {
    /// Display name, also shown in the status bar.
    pub name: &'static str,
    /// Extensions without the dot, lower case.
    pub extensions: &'static [&'static str],
    /// Whole file names (for example `Dockerfile`).
    pub file_names: &'static [&'static str],
    pub icon: IconName,
    /// A line ending with one of these characters is followed by one more
    /// indentation level.
    pub indent_after: &'static [char],
    /// A line starting with one of these words is followed by one level
    /// less of indentation.
    pub dedent_after: &'static [&'static str],
    /// Line comment prefix with its trailing space (`"# "`).
    pub line_comment: Option<&'static str>,
    /// Quotes closed automatically when typed.
    pub quotes: &'static [char],
    /// Three quotes open a triple-quoted string.
    pub triple_quotes: bool,
    /// Letters allowed as a string prefix right before a quote.
    pub string_prefixes: &'static [char],
    pub highlighter: Option<HighlighterFactory>,
    /// The language id language services know the files by (`"python"`);
    /// editor tabs of a type without one get no code intelligence.
    pub language_id: Option<&'static str>,
    /// Where a soft-wrapped line may wrap when no space is near (the
    /// language's line wrap strategy); empty for the rules of any text.
    pub wrap_rules: &'static [WrapRule],
}

impl std::fmt::Debug for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileType")
            .field("name", &self.name)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl FileType {
    pub const fn plain(
        name: &'static str,
        extensions: &'static [&'static str],
        icon: IconName,
    ) -> Self {
        Self {
            name,
            extensions,
            file_names: &[],
            icon,
            indent_after: &[],
            dedent_after: &[],
            line_comment: None,
            quotes: &[],
            triple_quotes: false,
            string_prefixes: &[],
            highlighter: None,
            language_id: None,
            wrap_rules: &[],
        }
    }

    /// The same type with a line comment prefix.
    pub const fn with_comment(mut self, prefix: &'static str) -> Self {
        self.line_comment = Some(prefix);
        self
    }

    pub fn matches(&self, path: &Path) -> bool {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        if self.file_names.iter().any(|n| *n == file) {
            return true;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        !ext.is_empty() && self.extensions.contains(&ext.as_str())
    }

    pub fn editor_config(&self) -> EditorConfig {
        EditorConfig {
            indent_after: self.indent_after.to_vec(),
            dedent_after: self.dedent_after.iter().map(|w| w.to_string()).collect(),
            line_comment: self.line_comment.map(str::to_string),
            quotes: self.quotes.to_vec(),
            triple_quotes: self.triple_quotes,
            string_prefixes: self.string_prefixes.to_vec(),
            wrap_rules: if self.wrap_rules.is_empty() {
                WrapRule::defaults()
            } else {
                self.wrap_rules.to_vec()
            },
            ..EditorConfig::default()
        }
    }
}

/// File types of the platform.
pub fn platform_types() -> Vec<FileType> {
    vec![
        FileType::plain("Text", &["txt", "text", "log"], IconName::FILE_TEXT),
        FileType::plain("JSON", &["json", "jsonc"], IconName("fileTypes/json")),
        FileType::plain("TOML", &["toml"], IconName("fileTypes/toml")).with_comment("# "),
        FileType::plain("YAML", &["yaml", "yml"], IconName("fileTypes/yaml")).with_comment("# "),
        FileType::plain(
            "Markdown",
            &["md", "markdown"],
            IconName("fileTypes/markdown"),
        ),
        FileType::plain("reStructuredText", &["rst"], IconName("fileTypes/rst")),
        FileType::plain("XML", &["xml", "xsd", "svg"], IconName("fileTypes/xml")),
        FileType::plain("HTML", &["html", "htm"], IconName("fileTypes/html")),
        FileType::plain("CSS", &["css", "scss"], IconName("fileTypes/css")),
        FileType::plain(
            "JavaScript",
            &["js", "mjs", "cjs", "ts"],
            IconName("fileTypes/javaScript"),
        )
        .with_comment("// "),
        FileType::plain(
            "Shell Script",
            &["sh", "bash", "zsh"],
            IconName("fileTypes/shell"),
        )
        .with_comment("# "),
        FileType::plain("CSV", &["csv", "tsv"], IconName("fileTypes/csv")),
        FileType::plain("SQL", &["sql"], IconName("fileTypes/sql")).with_comment("-- "),
        FileType::plain(
            "Config",
            &["ini", "cfg", "conf", "env", "properties"],
            IconName("fileTypes/config"),
        )
        .with_comment("# "),
        FileType::plain(
            "Image",
            &["png", "jpg", "jpeg", "gif", "ico", "webp"],
            IconName("fileTypes/image"),
        ),
        FileType::plain(
            "Archive",
            &["zip", "gz", "tar", "whl", "jar"],
            IconName("fileTypes/archive"),
        ),
        FileType {
            file_names: &["Dockerfile", "Containerfile"],
            ..FileType::plain("Dockerfile", &["dockerfile"], IconName("fileTypes/docker"))
                .with_comment("# ")
        },
        FileType {
            file_names: &[".gitignore", ".dockerignore"],
            ..FileType::plain("Ignore File", &[], IconName("fileTypes/gitignore"))
                .with_comment("# ")
        },
    ]
}

/// Product types first, then the platform's.
#[derive(Debug, Clone, Default)]
pub struct FileTypes {
    types: Vec<FileType>,
}

impl FileTypes {
    pub fn new(product: &[FileType]) -> Self {
        let mut types = product.to_vec();
        types.extend(platform_types());
        Self { types }
    }

    /// Every type, product types first.
    pub fn types(&self) -> &[FileType] {
        &self.types
    }

    pub fn for_path(&self, path: &Path) -> Option<&FileType> {
        self.types.iter().find(|t| t.matches(path))
    }

    pub fn icon(&self, path: &Path) -> IconName {
        self.for_path(path)
            .map_or(IconName::FILE_UNKNOWN, |t| t.icon)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_types_by_extension_and_name() {
        let product = [FileType {
            indent_after: &[':'],
            ..FileType::plain("Python", &["py", "pyi"], IconName("python/file"))
        }];
        let types = FileTypes::new(&product);
        assert_eq!(types.for_path(Path::new("a/b.PY")).unwrap().name, "Python");
        assert_eq!(
            types.for_path(Path::new("Dockerfile")).unwrap().name,
            "Dockerfile"
        );
        assert_eq!(types.icon(Path::new("x.unknown")), IconName::FILE_UNKNOWN);
        assert_eq!(
            types.icon(Path::new("pyproject.toml")),
            IconName("fileTypes/toml")
        );
        assert_eq!(
            types
                .for_path(Path::new("m.py"))
                .unwrap()
                .editor_config()
                .indent_after,
            [':']
        );
    }
}
