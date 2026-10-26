//! A KeePass database (KDBX 4) as a secret store.
//!
//! Layout inside the database: group [`root_group`], one subgroup per
//! `service`, one entry per `account` (title and user name = account,
//! password = secret). Every change re-reads the file first (another
//! process, or KeePassXC, may have written it) and saves atomically. Reads
//! reuse the decrypted database while the file is unchanged.
//!
//! Databases in older formats (KDBX 3.1, as written by some IDEs) are read
//! but never rewritten: saving would upgrade them to KDBX 4 and lock out
//! the program that owns them.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use keepass::DatabaseKey;
use keepass::config::{DatabaseConfig, DatabaseVersion, KdfConfig};
use keepass::db::{Database, EntryId, GroupId, Times, fields};
use keepass::error::{CryptographyError, DatabaseKeyError, DatabaseOpenError, DatabaseSaveError};

use crate::fsutil::atomic_write;
use crate::{Result, Secret, SecretError, SecretErrorKind, SecretStore};

/// Store name used in messages.
pub const STORE_NAME: &str = "KeePass database";
/// Group that holds every entry of the product.
pub fn root_group() -> &'static str {
    app_info::name()
}

fn err(kind: SecretErrorKind, message: impl Into<String>) -> SecretError {
    SecretError::new(kind, STORE_NAME, message)
}

/// What unlocks a database: a master password and an optional key file.
#[derive(Clone)]
pub struct KeePassKey {
    password: Secret,
    key_file: Option<PathBuf>,
}

impl KeePassKey {
    pub fn new(password: Secret) -> Self {
        Self {
            password,
            key_file: None,
        }
    }

    pub fn with_key_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.key_file = Some(path.into());
        self
    }

    pub fn key_file(&self) -> Option<&Path> {
        self.key_file.as_deref()
    }

    pub(crate) fn database_key(&self) -> Result<DatabaseKey> {
        let key = DatabaseKey::new().with_password(self.password.expose());
        let Some(path) = &self.key_file else {
            return Ok(key);
        };
        let mut file = fs::File::open(path).map_err(|e| {
            let kind = if e.kind() == io::ErrorKind::NotFound {
                SecretErrorKind::Unavailable
            } else {
                SecretErrorKind::Io
            };
            err(
                kind,
                format!("cannot read key file {}: {e}", path.display()),
            )
        })?;
        key.with_keyfile(&mut file).map_err(|e| {
            err(
                SecretErrorKind::Io,
                format!("cannot read key file {}: {e}", path.display()),
            )
        })
    }
}

impl std::fmt::Debug for KeePassKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeePassKey")
            .field("password", &self.password)
            .field("key_file", &self.key_file)
            .finish()
    }
}

/// Argon2id settings for databases created here. The default follows the
/// usual KeePass recommendation (64 MiB); lower values are for tests only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub iterations: u64,
    pub memory_bytes: u64,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            iterations: 3,
            memory_bytes: 64 * 1024 * 1024,
            parallelism: 2,
        }
    }
}

impl KdfParams {
    /// Database settings for a new file: KDBX 4, AES-256 outer cipher,
    /// ChaCha20 for protected fields, Argon2id key derivation.
    pub fn database_config(self) -> DatabaseConfig {
        let mut config = DatabaseConfig::default();
        // The Argon2 version type lives in a dependency of `keepass`; take it
        // from the default configuration (Argon2d, version 1.3).
        if let KdfConfig::Argon2 { version, .. } = config.kdf_config {
            config.kdf_config = KdfConfig::Argon2id {
                iterations: self.iterations,
                memory: self.memory_bytes,
                parallelism: self.parallelism,
                version,
            };
        }
        config
    }
}

/// How to open a database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeePassOptions {
    /// Create an empty database when the file does not exist.
    pub create: bool,
    /// Key derivation for a newly created database.
    pub kdf: KdfParams,
}

/// Size and modification time: enough to notice that another process
/// rewrote the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

struct Loaded {
    stamp: Stamp,
    db: Database,
}

/// A KDBX file used as a [`SecretStore`]. Calls block (key derivation and
/// file I/O); run them off the UI thread.
pub struct KeePassStore {
    path: PathBuf,
    key: KeePassKey,
    options: KeePassOptions,
    cache: Mutex<Option<Loaded>>,
}

impl std::fmt::Debug for KeePassStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeePassStore")
            .field("path", &self.path)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl KeePassStore {
    /// Open the database at `path`, checking the key right away. A missing
    /// file is created (empty, KDBX 4) only when `options.create` is set.
    pub fn open(
        path: impl Into<PathBuf>,
        key: KeePassKey,
        options: KeePassOptions,
    ) -> Result<Self> {
        let store = Self {
            path: path.into(),
            key,
            options,
            cache: Mutex::new(None),
        };
        let loaded = store.load_or_create()?;
        *store.lock() = Some(loaded);
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Format version of the file, e.g. `KDBX 4.1`.
    pub fn version(&self) -> Result<String> {
        self.with_current(|db| Ok(db.config.version.to_string()))
    }

    /// All `(service, account)` pairs kept by the product. Names only.
    pub fn list(&self) -> Result<Vec<(String, String)>> {
        self.with_current(|db| {
            let mut out = Vec::new();
            let Some(root) = child_group(db, db.root().id(), root_group()) else {
                return Ok(out);
            };
            let Some(root) = db.group(root) else {
                return Ok(out);
            };
            for service in root.groups() {
                for entry in service.entries() {
                    if let Some(title) = entry.get_title() {
                        out.push((service.name.clone(), title.to_owned()));
                    }
                }
            }
            out.sort();
            Ok(out)
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Loaded>> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn stamp(&self) -> Result<Option<Stamp>> {
        match fs::metadata(&self.path) {
            Ok(m) => Ok(Some(Stamp {
                len: m.len(),
                modified: m.modified().ok(),
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(err(
                SecretErrorKind::Io,
                format!("cannot read {}: {e}", self.path.display()),
            )),
        }
    }

    fn missing(&self) -> SecretError {
        err(
            SecretErrorKind::Unavailable,
            format!("database file not found: {}", self.path.display()),
        )
    }

    /// Read and decrypt the file as it is on disk now.
    fn load(&self) -> Result<Loaded> {
        let stamp = self.stamp()?.ok_or_else(|| self.missing())?;
        let data = fs::read(&self.path).map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                self.missing()
            } else {
                err(
                    SecretErrorKind::Io,
                    format!("cannot read {}: {e}", self.path.display()),
                )
            }
        })?;
        let db = Database::parse(&data, self.key.database_key()?).map_err(open_error)?;
        Ok(Loaded { stamp, db })
    }

    fn load_or_create(&self) -> Result<Loaded> {
        if self.stamp()?.is_some() || !self.options.create {
            return self.load();
        }
        let mut db = Database::with_config(self.options.kdf.database_config());
        db.meta.generator = Some(app_info::name().into());
        db.meta.database_name = Some(format!("{} secrets", app_info::name()));
        self.save(&db)?;
        let stamp = self.stamp()?.ok_or_else(|| self.missing())?;
        Ok(Loaded { stamp, db })
    }

    fn save(&self, db: &Database) -> Result<()> {
        let mut data = Vec::new();
        db.save(&mut data, self.key.database_key()?)
            .map_err(save_error)?;
        atomic_write(&self.path, &data, true).map_err(|e| {
            err(
                SecretErrorKind::Io,
                format!("cannot write {}: {e}", self.path.display()),
            )
        })
    }

    /// Run `f` on the current contents, re-reading the file only when it
    /// changed since the last read.
    fn with_current<T>(&self, f: impl FnOnce(&Database) -> Result<T>) -> Result<T> {
        let mut cache = self.lock();
        let stamp = self.stamp()?.ok_or_else(|| self.missing())?;
        if cache.as_ref().map(|c| c.stamp) != Some(stamp) {
            *cache = Some(self.load()?);
        }
        match cache.as_ref() {
            Some(loaded) => f(&loaded.db),
            None => Err(self.missing()),
        }
    }

    /// Re-read the file, apply `f`, save when it reports a change.
    fn modify<T>(&self, f: impl FnOnce(&mut Database) -> (T, bool)) -> Result<T> {
        let mut cache = self.lock();
        let mut loaded = self.load_or_create()?;
        let (value, changed) = f(&mut loaded.db);
        if changed {
            if !matches!(loaded.db.config.version, DatabaseVersion::KDB4(_)) {
                *cache = None;
                return Err(err(
                    SecretErrorKind::Format,
                    format!(
                        "{} is a {} database: it is read-only here because saving would \
                         convert it to KDBX 4; use a KDBX 4 database for {}",
                        self.path.display(),
                        loaded.db.config.version,
                        app_info::name()
                    ),
                ));
            }
            self.save(&loaded.db)?;
            loaded.stamp = self.stamp()?.ok_or_else(|| self.missing())?;
        }
        *cache = Some(loaded);
        Ok(value)
    }
}

fn child_group(db: &Database, parent: GroupId, name: &str) -> Option<GroupId> {
    let parent = db.group(parent)?;
    parent.groups().find(|g| g.name == name).map(|g| g.id())
}

fn ensure_group(db: &mut Database, parent: GroupId, name: &str) -> GroupId {
    if let Some(id) = child_group(db, parent, name) {
        return id;
    }
    let mut parent = db.group_mut(parent).expect("parent group exists");
    let mut group = parent.add_group();
    group.name = name.to_owned();
    group.id()
}

fn entry_by_title(db: &Database, group: GroupId, title: &str) -> Option<EntryId> {
    let group = db.group(group)?;
    group
        .entries()
        .find(|e| e.get_title() == Some(title))
        .map(|e| e.id())
}

fn find_entry(db: &Database, service: &str, account: &str) -> Option<(GroupId, EntryId)> {
    let root = child_group(db, db.root().id(), root_group())?;
    let group = child_group(db, root, service)?;
    entry_by_title(db, group, account).map(|e| (group, e))
}

pub(crate) fn open_error(e: DatabaseOpenError) -> SecretError {
    match e {
        DatabaseOpenError::Key(DatabaseKeyError::IncorrectKey) => {
            err(SecretErrorKind::Locked, "wrong master password or key file")
        }
        DatabaseOpenError::Key(DatabaseKeyError::EmptyKey) => {
            err(SecretErrorKind::Locked, "no master password or key file")
        }
        // KDBX 3 decrypts the whole payload before it can check the key; a
        // wrong key shows up as bad padding.
        DatabaseOpenError::Cryptography(CryptographyError::InvalidPadding(_)) => err(
            SecretErrorKind::Locked,
            "wrong master password or key file (or a damaged file)",
        ),
        DatabaseOpenError::Io(e) => err(SecretErrorKind::Io, e.to_string()),
        other => err(
            SecretErrorKind::Format,
            format!("not a readable KeePass database: {other}"),
        ),
    }
}

fn save_error(e: DatabaseSaveError) -> SecretError {
    match e {
        DatabaseSaveError::Io(e) => err(SecretErrorKind::Io, e.to_string()),
        other => err(
            SecretErrorKind::Format,
            format!("cannot encode the database: {other}"),
        ),
    }
}

impl SecretStore for KeePassStore {
    fn name(&self) -> &str {
        STORE_NAME
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<Secret>> {
        self.with_current(|db| {
            Ok(find_entry(db, service, account)
                .and_then(|(_, id)| db.entry(id))
                .and_then(|e| e.get_password().map(Secret::new)))
        })
    }

    fn set(&self, service: &str, account: &str, secret: &Secret) -> Result<()> {
        self.modify(|db| {
            let root = ensure_group(db, db.root().id(), root_group());
            let group = ensure_group(db, root, service);
            match entry_by_title(db, group, account) {
                Some(id) => {
                    let mut entry = db.entry_mut(id).expect("entry exists");
                    entry.set_unprotected(fields::USERNAME, account);
                    entry.set_protected(fields::PASSWORD, secret.expose());
                    entry.times.last_modification = Some(Times::now());
                }
                None => {
                    let mut group = db.group_mut(group).expect("group exists");
                    let mut entry = group.add_entry();
                    entry.set_unprotected(fields::TITLE, account);
                    entry.set_unprotected(fields::USERNAME, account);
                    entry.set_protected(fields::PASSWORD, secret.expose());
                }
            }
            ((), true)
        })
    }

    fn delete(&self, service: &str, account: &str) -> Result<bool> {
        self.modify(|db| {
            let Some((group, id)) = find_entry(db, service, account) else {
                return (false, false);
            };
            if let Some(entry) = db.entry_mut(id) {
                entry.remove();
            }
            let empty = db
                .group(group)
                .is_some_and(|g| g.entries().next().is_none() && g.groups().next().is_none());
            if empty && let Some(g) = db.group_mut(group) {
                g.remove();
            }
            (true, true)
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Cheap key derivation: unit tests run unoptimised.
    pub(crate) const FAST: KdfParams = KdfParams {
        iterations: 1,
        memory_bytes: 64 * 1024,
        parallelism: 1,
    };

    pub(crate) fn fast_create() -> KeePassOptions {
        KeePassOptions {
            create: true,
            kdf: FAST,
        }
    }

    fn key(pw: &str) -> KeePassKey {
        KeePassKey::new(Secret::new(pw))
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("secrets.kdbx");

        let missing = KeePassStore::open(&path, key("master"), KeePassOptions::default());
        assert_eq!(missing.unwrap_err().kind, SecretErrorKind::Unavailable);

        let store = KeePassStore::open(&path, key("master"), fast_create()).unwrap();
        assert!(path.exists());
        assert_eq!(store.version().unwrap(), "KDBX4.1");
        assert!(store.get("pyoxide-db", "prod").unwrap().is_none());

        store
            .set("pyoxide-db", "prod", &Secret::new("s3cret"))
            .unwrap();
        store
            .set("pyoxide-db", "prod/wallet", &Secret::new("w"))
            .unwrap();
        store.set("other", "prod", &Secret::new("o")).unwrap();
        assert_eq!(
            store.get("pyoxide-db", "prod").unwrap().unwrap().expose(),
            "s3cret"
        );
        store
            .set("pyoxide-db", "prod", &Secret::new("changed"))
            .unwrap();

        // A second handle (another process) sees the same data.
        let again = KeePassStore::open(&path, key("master"), KeePassOptions::default()).unwrap();
        assert_eq!(
            again.get("pyoxide-db", "prod").unwrap().unwrap().expose(),
            "changed"
        );
        assert_eq!(again.get("other", "prod").unwrap().unwrap().expose(), "o");
        assert_eq!(
            again.list().unwrap(),
            vec![
                ("other".to_owned(), "prod".to_owned()),
                ("pyoxide-db".to_owned(), "prod".to_owned()),
                ("pyoxide-db".to_owned(), "prod/wallet".to_owned()),
            ]
        );

        assert!(store.delete("pyoxide-db", "prod").unwrap());
        assert!(!store.delete("pyoxide-db", "prod").unwrap());
        assert!(store.delete("other", "prod").unwrap());
        // `again` notices the change made through `store`.
        assert!(again.get("pyoxide-db", "prod").unwrap().is_none());
        assert_eq!(
            again.list().unwrap(),
            vec![("pyoxide-db".to_owned(), "prod/wallet".to_owned())]
        );

        // Entry layout: Pyoxide / service / account.
        let db = Database::parse(
            &fs::read(&path).unwrap(),
            key("master").database_key().unwrap(),
        )
        .unwrap();
        let root = child_group(&db, db.root().id(), root_group()).unwrap();
        let svc = child_group(&db, root, "pyoxide-db").unwrap();
        let entry = db
            .entry(entry_by_title(&db, svc, "prod/wallet").unwrap())
            .unwrap();
        assert_eq!(entry.get_username(), Some("prod/wallet"));
        assert!(
            child_group(&db, root, "other").is_none(),
            "empty group removed"
        );
    }

    #[test]
    fn wrong_password_is_locked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.kdbx");
        KeePassStore::open(&path, key("right"), fast_create()).unwrap();
        let e = KeePassStore::open(&path, key("wrong"), KeePassOptions::default()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Locked);
        assert!(!e.to_string().contains("wrong\""));
    }

    #[test]
    fn key_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.kdbx");
        let key_path = dir.path().join("k.key");
        fs::write(&key_path, b"0123456789abcdef0123456789abcdef").unwrap();
        let with_file = || key("pw").with_key_file(&key_path);
        let store = KeePassStore::open(&path, with_file(), fast_create()).unwrap();
        store.set("s", "a", &Secret::new("x")).unwrap();

        let e = KeePassStore::open(&path, key("pw"), KeePassOptions::default()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Locked);
        let again = KeePassStore::open(&path, with_file(), KeePassOptions::default()).unwrap();
        assert_eq!(again.get("s", "a").unwrap().unwrap().expose(), "x");

        let missing = key("pw").with_key_file(dir.path().join("nope.key"));
        let e = KeePassStore::open(&path, missing, KeePassOptions::default()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Unavailable);
    }

    #[test]
    fn corrupt_file_is_format_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.kdbx");
        fs::write(&path, b"definitely not a keepass file").unwrap();
        let e = KeePassStore::open(&path, key("pw"), KeePassOptions::default()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Format);
        // `create` never overwrites an existing file.
        let e = KeePassStore::open(&path, key("pw"), fast_create()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Format);
        assert_eq!(fs::read(&path).unwrap(), b"definitely not a keepass file");
    }

    #[test]
    fn deleted_file_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone.kdbx");
        let store =
            KeePassStore::open(&path, key("pw"), KeePassOptions::default().with_create()).unwrap();
        fs::remove_file(&path).unwrap();
        let e = store.get("s", "a").unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Unavailable);
    }

    #[test]
    fn kdbx3_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.kdbx");
        let data = crate::test_kdbx3::write("pw", &[(root_group(), &[])]);
        fs::write(&path, &data).unwrap();

        let e = KeePassStore::open(&path, key("bad"), KeePassOptions::default()).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Locked);

        let store = KeePassStore::open(&path, key("pw"), KeePassOptions::default()).unwrap();
        assert_eq!(store.version().unwrap(), "KDBX3.1");
        assert!(store.get("svc", "a").unwrap().is_none());
        assert!(
            !store.delete("svc", "a").unwrap(),
            "nothing to delete, nothing written"
        );
        let e = store.set("svc", "a", &Secret::new("x")).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Format);
        assert_eq!(fs::read(&path).unwrap(), data, "file untouched");
    }

    #[test]
    fn debug_hides_master_password() {
        let k = key("hunter2").with_key_file("/k");
        assert!(!format!("{k:?}").contains("hunter2"));
    }

    #[test]
    fn default_kdf_is_argon2id_kdbx4() {
        let config = KdfParams::default().database_config();
        assert!(matches!(config.version, DatabaseVersion::KDB4(_)));
        assert!(matches!(
            config.kdf_config,
            KdfConfig::Argon2id {
                memory: 67_108_864,
                ..
            }
        ));
    }

    impl KeePassOptions {
        fn with_create(self) -> Self {
            Self {
                create: true,
                kdf: FAST,
            }
        }
    }
}
