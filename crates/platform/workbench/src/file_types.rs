//! File types: which files are of which type, their icons and the editor
//! settings for them. The platform knows common formats; a product adds its
//! languages and its own associations (`pyproject.toml` is TOML).
//!
//! A file's type follows the reference IDE's rules: its whole name
//! (case-sensitive), then its whole name in any case, then name patterns
//! (the most specific first), then its extension in any case. The
//! associations are data, a list of masks per type, so a settings page can
//! show and edit them; an association added later replaces the one of the
//! same mask, so a person's own associations override the built-in ones.
//! A file no association names is typed by its first bytes, read off the
//! UI thread ([`FileTypes::detect`]): a `#!` line naming a type's
//! interpreter, text, or binary content. Some files of a type then get an
//! icon of their own, as the reference's file icon providers give it
//! (GitLab CI and Docker Compose files are YAML).

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use editor::{EditorConfig, Highlighter, WrapRule};
use ui::icons::IconName;

/// Creates the highlighter of a file type for a new editor.
pub type HighlighterFactory = Arc<dyn Fn() -> Box<dyn Highlighter> + Send + Sync>;

/// The type of text files, also the type of a file no association names
/// whose first bytes are text.
pub const TEXT: &str = "Text";

/// A kind of file the product knows.
#[derive(Clone)]
pub struct FileType {
    /// Display name, also shown in the status bar.
    pub name: &'static str,
    /// Extensions without the dot, matched in any case.
    pub extensions: &'static [&'static str],
    /// Whole file names, case-sensitive (`Dockerfile`).
    pub file_names: &'static [&'static str],
    /// Whole file names matched in any case (`requirements.txt`).
    pub file_names_any_case: &'static [&'static str],
    /// Name patterns with `*` (any run of characters) and `?` (one
    /// character), case-sensitive (`.env.*`).
    pub patterns: &'static [&'static str],
    /// Words a `#!` first line holds in a file of this type whose name no
    /// association names (`python`).
    pub hash_bangs: &'static [&'static str],
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
            file_names_any_case: &[],
            patterns: &[],
            hash_bangs: &[],
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

    /// The type's built-in associations.
    pub fn matchers(&self) -> impl Iterator<Item = NameMatcher> + '_ {
        let names = self
            .file_names
            .iter()
            .map(|n| NameMatcher::Name(n.to_string()));
        let any_case = self
            .file_names_any_case
            .iter()
            .map(|n| NameMatcher::NameAnyCase(n.to_string()));
        let patterns = self.patterns.iter().map(|p| NameMatcher::parse(p));
        let extensions = self
            .extensions
            .iter()
            .map(|e| NameMatcher::Extension(e.to_lowercase()));
        names.chain(any_case).chain(patterns).chain(extensions)
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

/// How an association matches a file name; a mask in the reference's
/// settings (`*.py`, `Dockerfile`, `docker-compose*.yml`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NameMatcher {
    /// The whole name, case-sensitive.
    Name(String),
    /// The whole name in any case.
    NameAnyCase(String),
    /// The extension, the part after the last dot, in any case.
    Extension(String),
    /// A pattern with `*` and `?`, case-sensitive.
    Pattern(String),
}

impl NameMatcher {
    /// Reads a mask as the reference does: `*.` and an extension without
    /// wildcards or dots is an extension, a mask with `*` or `?` a pattern,
    /// anything else a whole name.
    pub fn parse(mask: &str) -> Self {
        if let Some(ext) = mask.strip_prefix("*.")
            && !ext.contains(['*', '?', '.'])
        {
            return Self::Extension(ext.to_lowercase());
        }
        if mask.contains(['*', '?']) {
            Self::Pattern(mask.to_owned())
        } else {
            Self::Name(mask.to_owned())
        }
    }

    /// The mask as settings show it.
    pub fn mask(&self) -> String {
        match self {
            Self::Name(n) | Self::NameAnyCase(n) | Self::Pattern(n) => n.clone(),
            Self::Extension(e) => format!("*.{e}"),
        }
    }

    pub fn accepts(&self, name: &str) -> bool {
        match self {
            Self::Name(n) => n == name,
            Self::NameAnyCase(n) => n.to_lowercase() == name.to_lowercase(),
            Self::Extension(e) => extension(name).to_lowercase() == *e,
            Self::Pattern(p) => project::wildcard_match(p, name),
        }
    }
}

/// The extension of a file name: the part after the last dot, empty
/// without one (`.gitignore` has the extension `gitignore`).
fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, ext)| ext)
}

/// File name associations with types (by type name), looked up in the
/// reference's order: whole names, whole names in any case, patterns (the
/// most specific first), extensions.
#[derive(Debug, Clone, Default)]
pub struct Associations {
    names: HashMap<String, &'static str>,
    /// By the lower-case name.
    names_any_case: HashMap<String, &'static str>,
    /// The most specific first.
    patterns: Vec<(String, &'static str)>,
    /// By the lower-case extension.
    extensions: HashMap<String, &'static str>,
}

impl Associations {
    /// Associates a mask with a type; returns the type the mask named
    /// before.
    pub fn add(&mut self, matcher: NameMatcher, file_type: &'static str) -> Option<&'static str> {
        match matcher {
            NameMatcher::Name(n) => self.names.insert(n, file_type),
            NameMatcher::NameAnyCase(n) => self.names_any_case.insert(n.to_lowercase(), file_type),
            NameMatcher::Extension(e) => self.extensions.insert(e.to_lowercase(), file_type),
            NameMatcher::Pattern(p) => {
                let old = match self.patterns.iter_mut().find(|(q, _)| *q == p) {
                    Some(entry) => Some(std::mem::replace(&mut entry.1, file_type)),
                    None => {
                        self.patterns.push((p, file_type));
                        None
                    }
                };
                self.patterns.sort_by_cached_key(|(p, _)| specificity(p));
                old
            }
        }
    }

    /// Removes a mask's association; returns the type it named.
    pub fn remove(&mut self, matcher: &NameMatcher) -> Option<&'static str> {
        match matcher {
            NameMatcher::Name(n) => self.names.remove(n),
            NameMatcher::NameAnyCase(n) => self.names_any_case.remove(&n.to_lowercase()),
            NameMatcher::Extension(e) => self.extensions.remove(&e.to_lowercase()),
            NameMatcher::Pattern(p) => {
                let i = self.patterns.iter().position(|(q, _)| q == p)?;
                Some(self.patterns.remove(i).1)
            }
        }
    }

    /// The type a file name is associated with.
    pub fn find(&self, name: &str) -> Option<&'static str> {
        if let Some(t) = self.names.get(name) {
            return Some(*t);
        }
        if !self.names_any_case.is_empty()
            && let Some(t) = self.names_any_case.get(&name.to_lowercase())
        {
            return Some(*t);
        }
        if let Some((_, t)) = self
            .patterns
            .iter()
            .find(|(p, _)| project::wildcard_match(p, name))
        {
            return Some(*t);
        }
        self.extensions
            .get(extension(name).to_lowercase().as_str())
            .copied()
    }

    /// The masks associated with a type: names, names in any case,
    /// patterns, extensions, each sorted.
    pub fn matchers_of(&self, file_type: &str) -> Vec<NameMatcher> {
        let sorted = |map: &HashMap<String, &'static str>| {
            let mut keys: Vec<String> = map
                .iter()
                .filter(|(_, t)| **t == file_type)
                .map(|(k, _)| k.clone())
                .collect();
            keys.sort();
            keys
        };
        let mut out: Vec<NameMatcher> = sorted(&self.names)
            .into_iter()
            .map(NameMatcher::Name)
            .collect();
        out.extend(
            sorted(&self.names_any_case)
                .into_iter()
                .map(NameMatcher::NameAnyCase),
        );
        let mut patterns: Vec<String> = self
            .patterns
            .iter()
            .filter(|(_, t)| *t == file_type)
            .map(|(p, _)| p.clone())
            .collect();
        patterns.sort();
        out.extend(patterns.into_iter().map(NameMatcher::Pattern));
        out.extend(
            sorted(&self.extensions)
                .into_iter()
                .map(NameMatcher::Extension),
        );
        out
    }
}

/// The order of patterns: longer ones first, then by text with `?` and
/// `*` after every other character.
fn specificity(pattern: &str) -> (Reverse<usize>, String) {
    let key = pattern.replace('?', "\u{FFFE}").replace('*', "\u{FFFF}");
    (Reverse(pattern.chars().count()), key)
}

/// A product's association of a file name mask with a type, its own or
/// the platform's (`uv.lock` is TOML).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileAssociation {
    pub mask: &'static str,
    pub file_type: &'static str,
}

/// What the first bytes of a file no association names say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected {
    /// The type a `#!` line names.
    Type(&'static str),
    /// Text, or an empty file.
    Text,
    /// Binary content: a file of unknown type.
    Binary,
}

/// How many bytes of a file are read to tell its type.
const DETECT_BYTES: u64 = 8 * 1024;

/// The type its first bytes give a file, by the reference's rules: a
/// Unicode byte order mark of UTF-16 or UTF-32 means text; a byte below 9
/// (the tab) means binary; a `#!` line holding a type's word names that
/// type; anything else is text.
fn content_type(bytes: &[u8], hash_bangs: &[(&'static str, &'static str)]) -> Detected {
    const BOMS: [&[u8]; 4] = [
        &[0x00, 0x00, 0xFE, 0xFF],
        &[0xFF, 0xFE, 0x00, 0x00],
        &[0xFE, 0xFF],
        &[0xFF, 0xFE],
    ];
    if BOMS.iter().any(|bom| bytes.starts_with(bom)) {
        return Detected::Text;
    }
    if bytes.iter().any(|&b| b < 9) {
        return Detected::Binary;
    }
    if let Some(rest) = bytes.strip_prefix(b"#!")
        && let Some(end) = rest.iter().position(|&b| b == b'\n')
    {
        let line = &rest[..end];
        let holds = |word: &str| line.windows(word.len()).any(|w| w == word.as_bytes());
        if let Some((_, t)) = hash_bangs.iter().find(|(word, _)| holds(word)) {
            return Detected::Type(t);
        }
    }
    Detected::Text
}

/// An icon some files of a type get in place of the type's own, as the
/// reference's file icon providers give it.
#[derive(Debug, Clone)]
struct IconRule {
    file_type: &'static str,
    /// The names it applies to.
    names: Vec<NameMatcher>,
    /// A further check of the whole path, for rules about folders.
    path: Option<fn(&Path) -> bool>,
    icon: IconName,
}

impl IconRule {
    fn new(
        file_type: &'static str,
        masks: &[&str],
        path: Option<fn(&Path) -> bool>,
        icon: IconName,
    ) -> Self {
        Self {
            file_type,
            names: masks.iter().map(|m| NameMatcher::parse(m)).collect(),
            path,
            icon,
        }
    }

    fn applies(&self, file_type: &str, name: &str, path: &Path) -> bool {
        self.file_type == file_type
            && self.names.iter().any(|m| m.accepts(name))
            && self.path.is_none_or(|f| f(path))
    }
}

/// A file in a GitHub Actions workflows folder: its path holds
/// `.github/` and, after it, `/workflows`.
fn in_github_workflows(path: &Path) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    match (path.find("/workflows"), path.find(".github/")) {
        (Some(workflows), Some(github)) => workflows > github,
        _ => false,
    }
}

/// Icons of some YAML files: GitLab CI and GitHub Actions files (the
/// reference's GitLab and GitHub plugins) and Docker Compose files
/// (observed); pre-commit configurations keep the YAML icon, as in the
/// reference.
fn platform_icon_rules() -> Vec<IconRule> {
    const GITHUB: IconName = IconName("vcs/vendors/github");
    vec![
        IconRule::new(
            "YAML",
            &[
                "*.gitlab-ci.yml",
                "*.gitlab-ci.yaml",
                "*.gitlab-ci.*.yml",
                "*.gitlab-ci.*.yaml",
            ],
            None,
            IconName("fileTypes/gitLabLogo"),
        ),
        IconRule::new("YAML", &["action.yml", "action.yaml"], None, GITHUB),
        IconRule::new(
            "YAML",
            &["*.yml", "*.yaml"],
            Some(in_github_workflows),
            GITHUB,
        ),
        IconRule::new(
            "YAML",
            &[
                "docker-compose*.yml",
                "docker-compose*.yaml",
                "compose.yml",
                "compose.yaml",
                "compose.*.yml",
                "compose.*.yaml",
            ],
            None,
            IconName("fileTypes/dockerCompose"),
        ),
    ]
}

/// The icon of the version control ignore files.
const IGNORED: IconName = IconName("fileTypes/ignored");

/// File types of the platform: the reference's file types of the
/// Community platform and the plugins it bundles, and the Docker and INI
/// files of the reference's own plugins as observed.
pub fn platform_types() -> Vec<FileType> {
    const JSON: IconName = IconName("fileTypes/json");
    const IMAGE: IconName = IconName("fileTypes/image");
    vec![
        FileType::plain(TEXT, &["txt", "text", "log"], IconName::FILE_TEXT),
        FileType::plain("JSON", &["json", "jsonc", "har"], JSON),
        FileType::plain("JSON5", &["json5"], JSON).with_comment("// "),
        FileType::plain(
            "JSON Lines",
            &["jsonl", "jsonlines", "ldjson", "ndjson"],
            JSON,
        ),
        FileType {
            file_names: &["Cargo.lock", "Cargo.toml.orig", "Gopkg.lock"],
            ..FileType::plain("TOML", &["toml"], IconName("fileTypes/toml")).with_comment("# ")
        },
        FileType::plain("YAML", &["yaml", "yml"], IconName("fileTypes/yaml")).with_comment("# "),
        FileType::plain(
            "Markdown",
            &["md", "markdown"],
            IconName("fileTypes/markdown"),
        ),
        FileType::plain("reStructuredText", &["rst"], IconName("fileTypes/rst")),
        FileType::plain(
            "XML",
            &[
                "xml", "xsd", "xsl", "xslt", "ant", "fxml", "jhm", "jnlp", "jrxml", "rng", "tld",
                "wsdl", "xul",
            ],
            IconName("fileTypes/xml"),
        ),
        FileType::plain("SVG", &["svg"], IMAGE),
        FileType::plain(
            "HTML",
            &["html", "htm", "sht", "shtm", "shtml"],
            IconName("fileTypes/html"),
        ),
        FileType::plain("CSS", &["css", "scss"], IconName("fileTypes/css")),
        FileType::plain(
            "JavaScript",
            &["js", "mjs", "cjs", "ts"],
            IconName("fileTypes/javaScript"),
        )
        .with_comment("// "),
        FileType {
            hash_bangs: &["bash", "sh", "zsh"],
            ..FileType::plain(
                "Shell Script",
                &["sh", "bash", "zsh"],
                IconName("fileTypes/shell"),
            )
            .with_comment("# ")
        },
        FileType::plain("CSV", &["csv", "tsv"], IconName("fileTypes/csv")),
        FileType::plain("SQL", &["sql"], IconName("fileTypes/sql")).with_comment("-- "),
        FileType::plain(
            "Properties",
            &["properties"],
            IconName("fileTypes/properties"),
        )
        .with_comment("# "),
        FileType::plain("INI", &["ini"], IconName("fileTypes/ini")).with_comment("# "),
        FileType {
            file_names: &[".editorconfig"],
            ..FileType::plain("EditorConfig", &[], IconName("fileTypes/editorConfig"))
                .with_comment("# ")
        },
        FileType {
            file_names: &[".env"],
            patterns: &[".env.*"],
            ..FileType::plain(".env file", &["env"], IconName::FILE_TEXT).with_comment("# ")
        },
        FileType::plain(
            "Image",
            &[
                "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "tif", "tiff", "icns", "psd",
                "tga", "bigtiff", "dcx", "jbig2", "pam", "pbm", "pcx", "pgm", "pnm", "ppm", "rgbe",
                "wbmp", "xbm", "xpm",
            ],
            IMAGE,
        ),
        FileType::plain(
            "Archive",
            &["zip", "gz", "tar", "whl", "jar"],
            IconName("fileTypes/archive"),
        ),
        FileType::plain("Patch", &["patch", "diff"], IconName("fileTypes/patch")),
        FileType::plain("Jupyter", &["ipynb"], IconName("fileTypes/jupyterNotebook")),
        FileType {
            file_names: &["Dockerfile", "Containerfile"],
            ..FileType::plain("Dockerfile", &["dockerfile"], IconName("fileTypes/docker"))
                .with_comment("# ")
        },
        FileType::plain(
            "DockerIgnore file",
            &["dockerignore"],
            IconName("fileTypes/dockerIgnore"),
        )
        .with_comment("# "),
        FileType::plain("GitIgnore file", &["gitignore"], IGNORED).with_comment("# "),
        FileType {
            file_names: &["exclude"],
            ..FileType::plain("GitExclude file", &[], IGNORED).with_comment("# ")
        },
        FileType::plain("HgIgnore file", &["hgignore"], IGNORED).with_comment("# "),
        FileType::plain("IgnoreLang file", &["ignore"], IGNORED).with_comment("# "),
    ]
}

/// The type of a name in [`FileTypes::associate`] is not a known type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownFileType(pub String);

/// The types, their associations and icon rules.
#[derive(Debug, Clone, Default)]
struct Inner {
    /// Product types first, then the platform's.
    types: Vec<FileType>,
    associations: Associations,
    icon_rules: Vec<IconRule>,
    /// `#!` words and the types they name, product types first.
    hash_bangs: Vec<(&'static str, &'static str)>,
}

/// The file types of a product and the platform, with their associations.
/// Cheap to clone; clones share what the first bytes of files said.
#[derive(Debug, Clone, Default)]
pub struct FileTypes {
    inner: Arc<Inner>,
    /// What the first bytes of files no association names said, by path.
    detected: Arc<Mutex<HashMap<PathBuf, Detected>>>,
}

impl FileTypes {
    pub fn new(product: &[FileType]) -> Self {
        Self::with_associations(product, &[])
    }

    /// The platform's types and the product's, then the product's own
    /// associations; a later association of a mask replaces an earlier
    /// one, so the product's replace the platform's.
    pub fn with_associations(product: &[FileType], associations: &[FileAssociation]) -> Self {
        let platform = platform_types();
        let mut table = Associations::default();
        for t in platform.iter().chain(product) {
            for matcher in t.matchers() {
                table.add(matcher, t.name);
            }
        }
        for a in associations {
            table.add(NameMatcher::parse(a.mask), a.file_type);
        }
        let mut types = product.to_vec();
        types.extend(platform);
        let hash_bangs = types
            .iter()
            .flat_map(|t| t.hash_bangs.iter().map(move |word| (*word, t.name)))
            .collect();
        Self {
            inner: Arc::new(Inner {
                types,
                associations: table,
                icon_rules: platform_icon_rules(),
                hash_bangs,
            }),
            detected: Arc::default(),
        }
    }

    /// Every type, product types first.
    pub fn types(&self) -> &[FileType] {
        &self.inner.types
    }

    /// The type of this name.
    pub fn by_name(&self, name: &str) -> Option<&FileType> {
        self.inner.types.iter().find(|t| t.name == name)
    }

    pub fn associations(&self) -> &Associations {
        &self.inner.associations
    }

    /// Associates a mask with a type, replacing what the mask named
    /// before (a person's association overrides a built-in one); returns
    /// that type.
    pub fn associate(
        &mut self,
        mask: &str,
        file_type: &str,
    ) -> Result<Option<&'static str>, UnknownFileType> {
        let name = self
            .by_name(file_type)
            .ok_or_else(|| UnknownFileType(file_type.to_owned()))?
            .name;
        let inner = Arc::make_mut(&mut self.inner);
        Ok(inner.associations.add(NameMatcher::parse(mask), name))
    }

    /// Removes a mask's association, a built-in one too; returns the type
    /// it named.
    pub fn remove_association(&mut self, mask: &str) -> Option<&'static str> {
        let inner = Arc::make_mut(&mut self.inner);
        inner.associations.remove(&NameMatcher::parse(mask))
    }

    /// The masks of a type, as settings list them.
    pub fn masks(&self, file_type: &str) -> Vec<String> {
        self.inner
            .associations
            .matchers_of(file_type)
            .iter()
            .map(NameMatcher::mask)
            .collect()
    }

    /// The type a file name is associated with.
    pub fn for_name(&self, name: &str) -> Option<&FileType> {
        self.by_name(self.inner.associations.find(name)?)
    }

    /// The type of a file: by its name, else by what its first bytes said
    /// ([`FileTypes::detect`]).
    pub fn for_path(&self, path: &Path) -> Option<&FileType> {
        let name = file_name(path);
        if let Some(t) = self.inner.associations.find(&name) {
            return self.by_name(t);
        }
        match self.detected(path)? {
            Detected::Type(t) => self.by_name(t),
            Detected::Text => self.by_name(TEXT),
            Detected::Binary => None,
        }
    }

    /// The icon of a file: an icon rule's, its type's, or for a file of no
    /// type the unknown icon when its content is binary and the text icon
    /// otherwise (also before its first bytes were read).
    pub fn icon(&self, path: &Path) -> IconName {
        match self.for_path(path) {
            Some(t) => {
                let name = file_name(path);
                self.inner
                    .icon_rules
                    .iter()
                    .find(|rule| rule.applies(t.name, &name, path))
                    .map_or(t.icon, |rule| rule.icon)
            }
            None if self.detected(path) == Some(Detected::Binary) => IconName::FILE_UNKNOWN,
            None => IconName::FILE_TEXT,
        }
    }

    /// What the first bytes of a file said, if they were read.
    pub fn detected(&self, path: &Path) -> Option<Detected> {
        let detected = self.detected.lock().unwrap_or_else(|e| e.into_inner());
        detected.get(path).copied()
    }

    /// Reads the first bytes of the regular files among `paths` that no
    /// association names and remembers what they say. Blocking: call it
    /// off the UI thread. Files read before are read again, their content
    /// may have changed.
    pub fn detect<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let mut found = Vec::new();
        for path in paths {
            if self.inner.associations.find(&file_name(path)).is_some() {
                continue;
            }
            if let Some(content) = self.read_content(path) {
                found.push((path.to_path_buf(), content));
            }
        }
        if !found.is_empty() {
            let mut detected = self.detected.lock().unwrap_or_else(|e| e.into_inner());
            detected.extend(found);
        }
    }

    /// What the first bytes of a file say about its type.
    pub fn content_type(&self, bytes: &[u8]) -> Detected {
        content_type(bytes, &self.inner.hash_bangs)
    }

    fn read_content(&self, path: &Path) -> Option<Detected> {
        // Only regular files: opening a named pipe would wait for a writer.
        if !std::fs::metadata(path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .ok()?
            .take(DETECT_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(self.content_type(&bytes))
    }
}

fn file_name(path: &Path) -> Cow<'_, str> {
    path.file_name()
        .map_or(Cow::Borrowed(""), |n| n.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python() -> FileType {
        FileType {
            indent_after: &[':'],
            hash_bangs: &["python"],
            ..FileType::plain("Python", &["py", "pyi"], IconName("python/file"))
        }
    }

    fn types() -> FileTypes {
        let product = [
            python(),
            FileType {
                file_names_any_case: &["requirements.txt"],
                ..FileType::plain("Requirements", &[], IconName::FILE_TEXT).with_comment("# ")
            },
        ];
        FileTypes::with_associations(
            &product,
            &[
                FileAssociation {
                    mask: "uv.lock",
                    file_type: "TOML",
                },
                FileAssociation {
                    mask: "pyproject.toml",
                    file_type: "TOML",
                },
            ],
        )
    }

    fn type_of(types: &FileTypes, name: &str) -> Option<&'static str> {
        types.for_path(Path::new(name)).map(|t| t.name)
    }

    #[test]
    fn masks_read_as_the_reference_reads_them() {
        assert_eq!(
            NameMatcher::parse("*.PY"),
            NameMatcher::Extension("py".into())
        );
        assert_eq!(
            NameMatcher::parse("*.tar.gz"),
            NameMatcher::Pattern("*.tar.gz".into())
        );
        assert_eq!(
            NameMatcher::parse("docker-compose*.yml"),
            NameMatcher::Pattern("docker-compose*.yml".into())
        );
        assert_eq!(
            NameMatcher::parse("Dockerfile"),
            NameMatcher::Name("Dockerfile".into())
        );
        assert_eq!(NameMatcher::parse("*"), NameMatcher::Pattern("*".into()));
        assert_eq!(NameMatcher::parse("*.py").mask(), "*.py");
        assert!(NameMatcher::parse("*.py").accepts("A.PY"));
        assert!(!NameMatcher::parse("Dockerfile").accepts("dockerfile"));
        assert!(NameMatcher::NameAnyCase("requirements.txt".into()).accepts("Requirements.TXT"));
        assert!(NameMatcher::parse(".env.*").accepts(".env.example"));
        assert!(!NameMatcher::parse(".env.*").accepts(".env"));
    }

    /// Whole name, then whole name in any case, then patterns, then the
    /// extension; patterns by specificity whatever their order.
    #[test]
    fn rule_order_is_name_then_pattern_then_extension() {
        let mut table = Associations::default();
        table.add(NameMatcher::parse("*.yml"), "YAML");
        table.add(NameMatcher::parse("*.TXT"), "Text");
        table.add(NameMatcher::parse("docker-compose*.yml"), "Compose");
        table.add(NameMatcher::parse("docker-compose.yml"), "Exact");
        table.add(
            NameMatcher::NameAnyCase("requirements.txt".into()),
            "Requirements",
        );
        table.add(NameMatcher::parse("requirements*.txt"), "Pattern");
        assert_eq!(table.find("docker-compose.yml"), Some("Exact"));
        assert_eq!(table.find("docker-compose.dev.yml"), Some("Compose"));
        assert_eq!(table.find("ci.YML"), Some("YAML"));
        assert_eq!(table.find("REQUIREMENTS.TXT"), Some("Requirements"));
        assert_eq!(table.find("requirements-dev.txt"), Some("Pattern"));
        assert_eq!(table.find("notes.txt"), Some("Text"));
        assert_eq!(table.find("Makefile"), None);

        // Longer patterns first, then by text with other characters before
        // `?` and `?` before `*`; not the order added.
        let mut table = Associations::default();
        table.add(NameMatcher::parse("a*"), "short");
        table.add(NameMatcher::parse("a*c"), "star");
        table.add(NameMatcher::parse("a?c"), "question");
        table.add(NameMatcher::parse("ab*"), "letter");
        assert_eq!(table.find("abc"), Some("letter"));
        assert_eq!(table.find("axc"), Some("question"));
        assert_eq!(table.find("axyc"), Some("star"));
        assert_eq!(table.find("axy"), Some("short"));
    }

    #[test]
    fn later_associations_replace_earlier_ones() {
        let mut types = types();
        // The product's association replaces nothing here, but reaches a
        // platform type.
        assert_eq!(type_of(&types, "uv.lock"), Some("TOML"));
        assert_eq!(type_of(&types, "Requirements.txt"), Some("Requirements"));
        assert_eq!(type_of(&types, "notes.txt"), Some("Text"));
        // A person's association overrides the built-in one of the mask.
        assert_eq!(types.associate("*.yml", "Text"), Ok(Some("YAML")));
        assert_eq!(type_of(&types, "ci.yml"), Some("Text"));
        assert_eq!(type_of(&types, "ci.yaml"), Some("YAML"));
        assert_eq!(types.associate("Makefile", "Shell Script"), Ok(None));
        assert_eq!(type_of(&types, "Makefile"), Some("Shell Script"));
        assert_eq!(
            types.associate("x", "Cobol"),
            Err(UnknownFileType("Cobol".into()))
        );
        assert_eq!(types.remove_association("*.yml"), Some("Text"));
        assert_eq!(type_of(&types, "ci.yml"), None);
        assert_eq!(types.masks("YAML"), ["*.yaml"]);
        assert_eq!(
            types.masks("TOML"),
            [
                "Cargo.lock",
                "Cargo.toml.orig",
                "Gopkg.lock",
                "pyproject.toml",
                "uv.lock",
                "*.toml"
            ]
        );
        assert_eq!(types.masks(".env file"), [".env", ".env.*", "*.env"]);
        // Clones made before keep their associations.
        assert_eq!(type_of(&self::types(), "ci.yml"), Some("YAML"));
    }

    /// The files the reference shows at the root of a Python project, with
    /// the icons it shows (names the associations do not know get theirs
    /// from their content, see `unknown_names_are_typed_by_content`).
    #[test]
    fn icons_of_a_python_project_root() {
        let types = types();
        let icon = |name: &str| types.icon(Path::new(name)).0;
        for (name, expected) in [
            ("app.py", "python/file"),
            ("stubs.pyi", "python/file"),
            (".dockerignore", "fileTypes/dockerIgnore"),
            ("Dockerfile", "fileTypes/docker"),
            ("docker-compose.yml", "fileTypes/dockerCompose"),
            ("docker-compose.override.yml", "fileTypes/dockerCompose"),
            ("compose.yaml", "fileTypes/dockerCompose"),
            (".env", "fileTypes/text"),
            (".env.example", "fileTypes/text"),
            (".gitignore", "fileTypes/ignored"),
            (".gitlab-ci.yml", "fileTypes/gitLabLogo"),
            (".pre-commit-config.yaml", "fileTypes/yaml"),
            ("alembic.ini", "fileTypes/ini"),
            ("tox.ini", "fileTypes/ini"),
            ("README.md", "fileTypes/markdown"),
            ("logs.log", "fileTypes/text"),
            ("pyproject.toml", "fileTypes/toml"),
            ("uv.lock", "fileTypes/toml"),
            ("requirements.txt", "fileTypes/text"),
            ("notebook.ipynb", "fileTypes/jupyterNotebook"),
            (".editorconfig", "fileTypes/editorConfig"),
            ("logging.properties", "fileTypes/properties"),
            ("fix.patch", "fileTypes/patch"),
            ("logo.svg", "fileTypes/image"),
            ("ci.yml", "fileTypes/yaml"),
            (".github/workflows/ci.yml", "vcs/vendors/github"),
            ("action.yaml", "vcs/vendors/github"),
            // Not read yet: text until the first bytes say otherwise.
            ("LICENSE", "fileTypes/text"),
        ] {
            assert_eq!(icon(name), expected, "{name}");
        }
        assert!(!in_github_workflows(Path::new("workflows/.github/ci.yml")));
        assert!(in_github_workflows(Path::new(
            r"C:\p\.github\workflows\ci.yml"
        )));
    }

    /// Icon rules follow the type: a GitLab CI file associated with Text
    /// shows the text icon.
    #[test]
    fn icon_rules_follow_the_type() {
        let mut types = types();
        types.associate(".gitlab-ci.yml", TEXT).unwrap();
        assert_eq!(types.icon(Path::new(".gitlab-ci.yml")), IconName::FILE_TEXT);
        assert_eq!(
            types.icon(Path::new("deploy.gitlab-ci.prod.yaml")).0,
            "fileTypes/gitLabLogo"
        );
    }

    #[test]
    fn first_bytes_tell_text_binary_and_scripts() {
        let types = types();
        assert_eq!(types.content_type(b""), Detected::Text);
        assert_eq!(types.content_type(b"3.13\n"), Detected::Text);
        assert_eq!(types.content_type("Ünïcode\n".as_bytes()), Detected::Text);
        assert_eq!(
            types.content_type(b"\x7fELF\x02\x01\x01\x00"),
            Detected::Binary
        );
        assert_eq!(types.content_type(b"\xff\xfeh\x00i\x00"), Detected::Text);
        assert_eq!(
            types.content_type(b"#!/usr/bin/env python3\nprint()\n"),
            Detected::Type("Python")
        );
        assert_eq!(
            types.content_type(b"#!/bin/bash\necho\n"),
            Detected::Type("Shell Script")
        );
        // The reference wants the line to end.
        assert_eq!(types.content_type(b"#!/bin/sh"), Detected::Text);
        assert_eq!(types.content_type(b"#!/usr/bin/perl\n"), Detected::Text);
    }

    #[test]
    fn unknown_names_are_typed_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let version = file(".python-version", b"3.13\n");
        let dev = file("Dockerfile.dev", b"FROM python:3.13\n");
        let empty = file("LICENSE", b"");
        let binary = file("data.bin", b"\x00\x01\x02");
        let script = file("manage", b"#!/usr/bin/env python\nimport sys\n");
        let readme = file("README.md", b"\x00");
        let types = types();
        let all = [&version, &dev, &empty, &binary, &script, &readme];
        types.detect(all.iter().map(|p| p.as_path()));
        assert_eq!(types.icon(&version), IconName::FILE_TEXT);
        assert_eq!(types.for_path(&version).map(|t| t.name), Some(TEXT));
        assert_eq!(types.icon(&dev), IconName::FILE_TEXT);
        assert_eq!(types.icon(&empty), IconName::FILE_TEXT);
        assert_eq!(types.icon(&binary), IconName::FILE_UNKNOWN);
        assert_eq!(types.for_path(&binary).map(|t| t.name), None);
        assert_eq!(types.for_path(&script).map(|t| t.name), Some("Python"));
        assert_eq!(types.icon(&script).0, "python/file");
        // A known name is never read.
        assert_eq!(types.detected(&readme), None);
        assert_eq!(types.icon(&readme).0, "fileTypes/markdown");
        // Clones share what was read; content read again replaces it.
        let clone = types.clone();
        std::fs::write(&binary, b"text now\n").unwrap();
        clone.detect([binary.as_path()]);
        assert_eq!(types.icon(&binary), IconName::FILE_TEXT);
    }

    #[test]
    fn editor_settings_follow_the_type() {
        let types = types();
        assert_eq!(
            types
                .for_path(Path::new("m.py"))
                .unwrap()
                .editor_config()
                .indent_after,
            [':']
        );
        assert_eq!(
            types
                .for_path(Path::new(".env.local"))
                .unwrap()
                .line_comment,
            Some("# ")
        );
        assert_eq!(type_of(&types, "a/b.PY"), Some("Python"));
    }
}
