//! Documents, file changes and the opaque tokens implementations attach to
//! model values.

use std::any::Any;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Serialize, Serializer};

/// A document: a file open in an editor, by its absolute path. Cheap to
/// clone.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentId(Arc<Path>);

impl DocumentId {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(Arc::from(path.into()))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl From<&Path> for DocumentId {
    fn from(path: &Path) -> Self {
        Self(Arc::from(path))
    }
}

impl From<PathBuf> for DocumentId {
    fn from(path: PathBuf) -> Self {
        Self::new(path)
    }
}

impl fmt::Debug for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DocumentId({})", self.0.display())
    }
}

impl Serialize for DocumentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0.display())
    }
}

/// What happened to a file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileChangeKind {
    Created,
    Changed,
    Deleted,
}

/// A change of a file on disk that the application made. A rename is the
/// old path deleted and the new one created.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
}

impl FileChange {
    pub fn new(path: impl Into<PathBuf>, kind: FileChangeKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

/// Data of an implementation attached to a model value, which the caller
/// hands back unchanged: the server and protocol data of a diagnostic sent
/// back with a code action request, of a completion item or code action to
/// resolve, of a command to run. Opaque to everyone else; two tokens are
/// equal when they are the same token.
#[derive(Clone)]
pub struct Token(Arc<dyn Any + Send + Sync>);

impl Token {
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// The value, when it is a `T` (for the implementation that made it).
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_and_tokens() {
        let a = DocumentId::new("/p/a.py");
        assert_eq!(a, DocumentId::from(Path::new("/p/a.py")));
        assert_eq!(a.path(), Path::new("/p/a.py"));
        assert_eq!(serde_json::to_value(&a).unwrap(), "/p/a.py");

        let token = Token::new(42u32);
        assert_eq!(token.downcast_ref::<u32>(), Some(&42));
        assert_eq!(token.downcast_ref::<u8>(), None);
        assert_eq!(token.clone(), token);
        assert_ne!(Token::new(42u32), token);
        assert_eq!(format!("{token:?}"), "Token(..)");
    }
}
