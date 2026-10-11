//! Read-only access to the credential store of a JetBrains IDE (PyCharm),
//! used to import passwords the user saved there.
//!
//! The formats follow the IntelliJ Platform Community source (Apache-2.0,
//! `platform/credential-store` and `platform/credential-store-impl`):
//! - settings: `<config dir>/options/security.xml`, component
//!   `PasswordSafe`, options `PROVIDER` (`KEYCHAIN`, `KEEPASS`,
//!   `MEMORY_ONLY`) and `keepassDb`; the default is KeePass on Windows and
//!   the OS store elsewhere;
//! - KeePass: `<config dir>/c.kdbx` unless `keepassDb` says otherwise,
//!   group `IntelliJ Platform`, entry title = service name, user name =
//!   account;
//! - master key: `<config dir>/c.pwd`, a small YAML file whose `value` is
//!   AES-128-CBC encrypted with a key built into the IDE (`BUILT_IN`), in
//!   addition wrapped with Windows DPAPI (`CRYPT_32`), or PGP-encrypted
//!   (`PGP_KEY`, not supported here);
//! - service names: `IntelliJ Platform <subsystem> — <key>`.
//!
//! Nothing here writes to the IDE's files or prints secret values.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use keepass::DatabaseKey;
use keepass::db::{Database, GroupId};
use quick_xml::events::Event;
use zeroize::Zeroize;

use crate::{Result, Secret, SecretError, SecretErrorKind, SecretStore};

/// Store name used in messages.
pub const STORE_NAME: &str = "IDE credential store";
/// Prefix of every service name, also the KeePass group name.
pub const SERVICE_NAME_PREFIX: &str = "IntelliJ Platform";
/// Default KeePass database in the IDE config directory.
pub const DB_FILE_NAME: &str = "c.kdbx";
/// Encrypted master key of the KeePass database, next to it.
pub const MAIN_KEY_FILE_NAME: &str = "c.pwd";
/// Master key file of older IDE versions (raw encrypted bytes).
pub const OLD_MAIN_KEY_FILE_NAME: &str = "pdb.pwd";
/// Password settings, relative to the config directory.
pub const SETTINGS_FILE: &str = "options/security.xml";
/// Config directory name prefixes of PyCharm (Professional / unified, Community).
pub const PYCHARM_PRODUCTS: &[&str] = &["PyCharm", "PyCharmCE"];

/// AES key the IDE uses for `BUILT_IN` master key encryption (16 ASCII
/// bytes, from the Community source). It only obfuscates: anyone who can
/// read `c.pwd` can decrypt it, so the file's protection is its location.
const BUILT_IN_KEY: [u8; 16] = *b"Proxy Config Sec";

fn err(kind: SecretErrorKind, message: impl Into<String>) -> SecretError {
    SecretError::new(kind, STORE_NAME, message)
}

/// `IntelliJ Platform <subsystem> — <key>`, the IDE's service naming scheme.
pub fn generate_service_name(subsystem: &str, key: &str) -> String {
    format!("{SERVICE_NAME_PREFIX} {subsystem} \u{2014} {key}")
}

/// Operating system conventions to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        }
    }
}

// ---------------------------------------------------------------- config dirs

/// One IDE configuration directory, e.g. `.../JetBrains/PyCharm2025.2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeConfigDir {
    /// Product prefix, e.g. `PyCharm` or `PyCharmCE`.
    pub product: String,
    /// Version part of the directory name, e.g. `2025.2`.
    pub version: String,
    pub path: PathBuf,
}

/// `JetBrains` under the OS config directory: `~/Library/Application
/// Support/JetBrains` (macOS), `%APPDATA%\JetBrains` (Windows),
/// `$XDG_CONFIG_HOME/JetBrains` or `~/.config/JetBrains` (Linux).
pub fn jetbrains_config_root() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("JetBrains"))
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = s.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() >= 2 && p[0] >= 2000)
}

/// Config directories under `root` whose name is one of `products`
/// followed by a version, newest first.
pub fn find_config_dirs(root: &Path, products: &[&str]) -> Vec<IdeConfigDir> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(Vec<u32>, usize, IdeConfigDir)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        for (rank, product) in products.iter().enumerate() {
            if let Some(version) = name.strip_prefix(product)
                && let Some(key) = parse_version(version)
            {
                found.push((
                    key,
                    rank,
                    IdeConfigDir {
                        product: (*product).to_owned(),
                        version: version.to_owned(),
                        path: entry.path(),
                    },
                ));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, _, d)| d).collect()
}

/// PyCharm config directories on this machine, newest first.
pub fn pycharm_config_dirs() -> Vec<IdeConfigDir> {
    jetbrains_config_root()
        .map(|root| find_config_dirs(&root, PYCHARM_PRODUCTS))
        .unwrap_or_default()
}

// ------------------------------------------------------------------ settings

/// Which store the IDE keeps passwords in (`PROVIDER`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderType {
    Keychain,
    KeePass,
    MemoryOnly,
    Other(String),
}

impl ProviderType {
    fn parse(s: &str) -> Self {
        match s {
            "KEYCHAIN" => ProviderType::Keychain,
            "KEEPASS" => ProviderType::KeePass,
            "MEMORY_ONLY" | "DO_NOT_STORE" => ProviderType::MemoryOnly,
            other => ProviderType::Other(other.to_owned()),
        }
    }
}

/// The `PasswordSafe` component of `options/security.xml`, as written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PasswordSafeSettings {
    /// `None` when the IDE uses its default.
    pub provider: Option<ProviderType>,
    /// Custom KeePass file; may contain path macros.
    pub keepass_db: Option<String>,
}

/// Parse `security.xml`.
pub fn parse_password_safe_settings(xml: &str) -> Result<PasswordSafeSettings> {
    let bad =
        |e: &dyn std::fmt::Display| err(SecretErrorKind::Format, format!("{SETTINGS_FILE}: {e}"));
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut settings = PasswordSafeSettings::default();
    let mut depth = 0usize;
    let mut component_depth: Option<usize> = None;
    loop {
        let event = reader.read_event().map_err(|e| bad(&e))?;
        let (start, empty) = match &event {
            Event::Start(s) => (Some(s), false),
            Event::Empty(s) => (Some(s), true),
            Event::End(_) => {
                if component_depth == Some(depth) {
                    component_depth = None;
                }
                depth = depth.saturating_sub(1);
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let Some(start) = start else { continue };
        let attr = |name: &str| -> Result<Option<String>> {
            match start.try_get_attribute(name).map_err(|e| bad(&e))? {
                Some(a) => Ok(Some(
                    a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| bad(&e))?
                        .into_owned(),
                )),
                None => Ok(None),
            }
        };
        let tag = start.name();
        if !empty {
            depth += 1;
        }
        if tag.as_ref() == "component" && attr("name")?.as_deref() == Some("PasswordSafe") {
            if !empty {
                component_depth = Some(depth);
            }
        } else if tag.as_ref() == "option" && component_depth.is_some() {
            let value = attr("value")?;
            match attr("name")?.as_deref() {
                Some("PROVIDER") => settings.provider = value.as_deref().map(ProviderType::parse),
                Some("keepassDb") => settings.keepass_db = value.filter(|v| !v.trim().is_empty()),
                _ => {}
            }
        }
    }
    Ok(settings)
}

/// Read `<config dir>/options/security.xml`; a missing file means defaults.
pub fn read_password_safe_settings(config_dir: &Path) -> Result<PasswordSafeSettings> {
    let path = config_dir.join(SETTINGS_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_password_safe_settings(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PasswordSafeSettings::default()),
        Err(e) => Err(err(
            SecretErrorKind::Io,
            format!("cannot read {}: {e}", path.display()),
        )),
    }
}

/// Where the IDE keeps passwords, resolved for one config directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdeStore {
    KeePass {
        db_file: PathBuf,
        main_key_file: PathBuf,
    },
    /// macOS Keychain or Secret Service.
    OsStore,
    /// Passwords are not saved at all.
    MemoryOnly,
    Unsupported(String),
}

/// Expand the application path macros the IDE writes into settings values.
fn expand_app_macros(value: &str, config_dir: &Path) -> PathBuf {
    let config = config_dir.to_string_lossy().replace('\\', "/");
    let mut s = value
        .replace("$APPLICATION_CONFIG_DIR$", &config)
        .replace("$APP_CONFIG$", &config);
    if let Some(home) = dirs::home_dir() {
        s = s.replace("$USER_HOME$", &home.to_string_lossy().replace('\\', "/"));
    }
    PathBuf::from(s)
}

/// Apply the IDE's rules to its settings: the default provider is KeePass
/// on Windows (where the OS store is never used) and the OS store elsewhere.
pub fn resolve_store(
    config_dir: &Path,
    settings: &PasswordSafeSettings,
    platform: Platform,
) -> IdeStore {
    let default = if platform == Platform::Windows {
        ProviderType::KeePass
    } else {
        ProviderType::Keychain
    };
    let mut provider = settings.provider.clone().unwrap_or(default);
    if platform == Platform::Windows && provider == ProviderType::Keychain {
        provider = ProviderType::KeePass;
    }
    match provider {
        ProviderType::KeePass => IdeStore::KeePass {
            db_file: settings
                .keepass_db
                .as_deref()
                .map(|v| expand_app_macros(v, config_dir))
                .unwrap_or_else(|| config_dir.join(DB_FILE_NAME)),
            main_key_file: config_dir.join(MAIN_KEY_FILE_NAME),
        },
        ProviderType::Keychain => IdeStore::OsStore,
        ProviderType::MemoryOnly => IdeStore::MemoryOnly,
        ProviderType::Other(p) => IdeStore::Unsupported(format!("password provider {p}")),
    }
}

/// Read the settings of `config_dir` and resolve the store for this OS.
pub fn detect_store(config_dir: &Path) -> Result<IdeStore> {
    let settings = read_password_safe_settings(config_dir)?;
    Ok(resolve_store(config_dir, &settings, Platform::current()))
}

// ---------------------------------------------------------------- master key

/// How the IDE protects the KeePass master key in `c.pwd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainKeyEncryption {
    /// AES with the key built into the IDE (macOS, Linux default).
    BuiltIn,
    /// Built-in AES, then Windows DPAPI (Windows default).
    Crypt32,
    /// The user's PGP key (gpg).
    PgpKey,
}

impl MainKeyEncryption {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "BUILT_IN" => Some(Self::BuiltIn),
            "CRYPT_32" => Some(Self::Crypt32),
            "PGP_KEY" => Some(Self::PgpKey),
            _ => None,
        }
    }

    /// The IDE's default when `c.pwd` does not name one.
    pub fn default_for(platform: Platform) -> Self {
        if platform == Platform::Windows {
            Self::Crypt32
        } else {
            Self::BuiltIn
        }
    }
}

/// `c.pwd`, parsed but still encrypted.
#[derive(Clone, PartialEq, Eq)]
pub struct MainKeyFile {
    pub encryption: MainKeyEncryption,
    value: Vec<u8>,
}

impl std::fmt::Debug for MainKeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainKeyFile")
            .field("encryption", &self.encryption)
            .finish_non_exhaustive()
    }
}

/// Parse `c.pwd`: `encryption: <type>`, `isAutoGenerated: <bool>`,
/// `value: !!binary <base64>` (YAML; the value may continue on indented
/// lines).
pub fn parse_main_key_file(content: &[u8], platform: Platform) -> Result<MainKeyFile> {
    let bad = |m: &str| {
        err(
            SecretErrorKind::Format,
            format!("{MAIN_KEY_FILE_NAME}: {m}"),
        )
    };
    let text = std::str::from_utf8(content).map_err(|_| bad("not UTF-8 text"))?;
    let mut encryption = None;
    let mut value: Option<String> = None;
    let mut in_value = false;
    for line in text.lines() {
        if in_value && line.starts_with([' ', '\t']) {
            value.get_or_insert_with(String::new).push_str(line.trim());
            continue;
        }
        in_value = false;
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let rest = rest.trim().trim_matches(['"', '\'']);
        match key.trim() {
            "encryption" => {
                encryption = Some(
                    MainKeyEncryption::parse(rest).ok_or_else(|| bad("unknown encryption type"))?,
                )
            }
            "value" => {
                let rest = rest.strip_prefix("!!binary").unwrap_or(rest).trim();
                let rest = rest.trim_start_matches(['|', '>']).trim();
                value = Some(rest.to_owned());
                in_value = true;
            }
            _ => {}
        }
    }
    let value = value
        .filter(|v| !v.is_empty())
        .ok_or_else(|| bad("no value"))?;
    let compact: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    let value = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| bad("value is not base64"))?;
    Ok(MainKeyFile {
        encryption: encryption.unwrap_or(MainKeyEncryption::default_for(platform)),
        value,
    })
}

/// `[u32 big-endian IV length][IV][AES-128-CBC, PKCS#7]` with the built-in key.
fn decrypt_built_in(data: &[u8]) -> Result<Vec<u8>> {
    let bad = || {
        err(
            SecretErrorKind::Format,
            "master key: malformed encrypted data",
        )
    };
    let len_bytes: [u8; 4] = data
        .get(..4)
        .ok_or_else(bad)?
        .try_into()
        .map_err(|_| bad())?;
    let iv_len = u32::from_be_bytes(len_bytes) as usize;
    if iv_len != 16 {
        return Err(bad());
    }
    let iv = data.get(4..4 + iv_len).ok_or_else(bad)?;
    let body = &data[4 + iv_len..];
    cbc::Decryptor::<aes::Aes128>::new_from_slices(&BUILT_IN_KEY, iv)
        .map_err(|_| bad())?
        .decrypt_padded_vec::<Pkcs7>(body)
        .map_err(|_| {
            err(
                SecretErrorKind::Format,
                "master key: cannot decrypt (not written by this IDE family?)",
            )
        })
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    let mut input = data.to_vec();
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let mut out_blob = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: both blobs point to valid memory for the duration of the
    // call; the output buffer is allocated by the OS and released below.
    let ok = unsafe {
        CryptUnprotectData(
            &in_blob,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut out_blob,
        )
    };
    input.zeroize();
    if ok == 0 {
        return Err(err(
            SecretErrorKind::Unavailable,
            format!(
                "master key: Windows DPAPI cannot decrypt it (another user or machine?): {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    // SAFETY: on success the OS returns `cbData` readable bytes at `pbData`.
    let out = unsafe {
        let bytes = std::slice::from_raw_parts_mut(out_blob.pbData, out_blob.cbData as usize);
        let copy = bytes.to_vec();
        bytes.zeroize();
        LocalFree(out_blob.pbData as HLOCAL);
        copy
    };
    Ok(out)
}

#[cfg(not(windows))]
fn dpapi_unprotect(_data: &[u8]) -> Result<Vec<u8>> {
    Err(err(
        SecretErrorKind::Unavailable,
        "master key is protected with Windows DPAPI: it can only be read on Windows by the \
         same user; enter the KeePass master password instead",
    ))
}

fn to_secret(mut bytes: Vec<u8>) -> Result<Secret> {
    match String::from_utf8(std::mem::take(&mut bytes)) {
        Ok(s) => Ok(Secret::new(s)),
        Err(e) => {
            e.into_bytes().zeroize();
            Err(err(
                SecretErrorKind::Format,
                "master key is not text; enter the KeePass master password instead",
            ))
        }
    }
}

/// Decrypt the master key of a parsed `c.pwd`.
pub fn decrypt_main_key(file: &MainKeyFile) -> Result<Secret> {
    match file.encryption {
        MainKeyEncryption::BuiltIn => to_secret(decrypt_built_in(&file.value)?),
        MainKeyEncryption::Crypt32 => {
            let mut inner = dpapi_unprotect(&file.value)?;
            let plain = decrypt_built_in(&inner);
            inner.zeroize();
            to_secret(plain?)
        }
        MainKeyEncryption::PgpKey => Err(err(
            SecretErrorKind::Unavailable,
            "master key is encrypted with a PGP key, which is not supported; enter the \
             KeePass master password instead",
        )),
    }
}

/// Parse and decrypt the content of `c.pwd`.
pub fn decode_main_key(content: &[u8], platform: Platform) -> Result<Secret> {
    decrypt_main_key(&parse_main_key_file(content, platform)?)
}

/// Decrypt the content of `pdb.pwd` (older IDE versions: raw bytes,
/// built-in key, plus DPAPI on Windows).
pub fn decode_old_main_key(content: &[u8], platform: Platform) -> Result<Secret> {
    let encryption = if platform == Platform::Windows {
        MainKeyEncryption::Crypt32
    } else {
        MainKeyEncryption::BuiltIn
    };
    decrypt_main_key(&MainKeyFile {
        encryption,
        value: content.to_vec(),
    })
}

/// The KeePass master key saved by the IDE in `config_dir`, if any.
pub fn read_main_key(config_dir: &Path) -> Result<Option<Secret>> {
    let platform = Platform::current();
    let read = |name: &str| match std::fs::read(config_dir.join(name)) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(err(SecretErrorKind::Io, format!("cannot read {name}: {e}"))),
    };
    if let Some(data) = read(MAIN_KEY_FILE_NAME)? {
        return decode_main_key(&data, platform).map(Some);
    }
    if let Some(data) = read(OLD_MAIN_KEY_FILE_NAME)? {
        return decode_old_main_key(&data, platform).map(Some);
    }
    Ok(None)
}

// ------------------------------------------------------------------- KeePass

/// A credential found in the IDE store.
#[derive(Debug, Clone)]
pub struct IdeCredential {
    pub user: Option<String>,
    pub password: Option<Secret>,
}

/// The IDE's KeePass database, opened read-only.
pub struct IdeKeePass {
    db: Database,
}

impl std::fmt::Debug for IdeKeePass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IdeKeePass").finish_non_exhaustive()
    }
}

impl IdeKeePass {
    /// Open with the master key (from [`read_main_key`] or typed by the
    /// user). The IDE writes KDBX 3.1; KDBX 4 also works.
    pub fn open(path: &Path, master: &Secret) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| {
            let kind = if e.kind() == std::io::ErrorKind::NotFound {
                SecretErrorKind::Unavailable
            } else {
                SecretErrorKind::Io
            };
            err(kind, format!("cannot read {}: {e}", path.display()))
        })?;
        let key = DatabaseKey::new().with_password(master.expose());
        let db = Database::parse(&data, key).map_err(|e| {
            let mut e = crate::keepass::open_error(e);
            e.store = STORE_NAME.to_owned();
            e
        })?;
        Ok(Self { db })
    }

    fn ide_group(&self) -> Option<GroupId> {
        let root = self.db.root();
        root.groups()
            .find(|g| g.name == SERVICE_NAME_PREFIX)
            .map(|g| g.id())
    }

    /// Look up like the IDE: entry title = `service_name`, user name =
    /// `user` when given. If no entry has that user name but exactly one
    /// has the title, that one is returned (the IDE keeps one credential
    /// per service).
    pub fn get(&self, service_name: &str, user: Option<&str>) -> Option<IdeCredential> {
        let group = self.db.group(self.ide_group()?)?;
        let with_title: Vec<_> = group
            .entries()
            .filter(|e| e.get_title() == Some(service_name))
            .collect();
        let user = user.filter(|u| !u.is_empty());
        let entry = match user {
            Some(u) => with_title
                .iter()
                .find(|e| e.get_username() == Some(u))
                .or_else(|| (with_title.len() == 1).then(|| &with_title[0])),
            None => with_title.first(),
        }?;
        Some(IdeCredential {
            user: entry
                .get_username()
                .filter(|u| !u.is_empty())
                .map(str::to_owned),
            password: entry.get_password().map(Secret::new),
        })
    }

    /// Entry titles in the IDE group that start with `prefix`, sorted.
    /// Titles are service names, never secrets; use this to confirm the
    /// naming a plugin uses.
    pub fn titles_with_prefix(&self, prefix: &str) -> Vec<String> {
        let Some(group) = self.ide_group().and_then(|g| self.db.group(g)) else {
            return Vec::new();
        };
        let mut titles: Vec<String> = group
            .entries()
            .filter_map(|e| e.get_title().map(str::to_owned))
            .filter(|t| t.starts_with(prefix))
            .collect();
        titles.sort();
        titles.dedup();
        titles
    }

    /// Number of entries in the IDE group.
    pub fn len(&self) -> usize {
        self.ide_group()
            .and_then(|g| self.db.group(g))
            .map(|g| g.entries().count())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ------------------------------------------------------------------ OS store

/// Whether passwords the IDE saved in the OS store can be read on
/// `platform`. `Err` explains why not.
pub fn os_store_support(platform: Platform) -> std::result::Result<(), String> {
    match platform {
        Platform::MacOs => Ok(()),
        Platform::Windows => Err("on Windows the IDE keeps passwords in KeePass, not in the \
                                  Credential Manager"
            .into()),
        Platform::Linux => Err("the IDE's Secret Service items use a 'service'/'account' \
                                attribute schema that this keyring backend cannot query; \
                                switch the IDE to KeePass to import, or enter the passwords"
            .into()),
    }
}

/// Read one IDE password from the OS credential store (pass
/// [`KeyringStore`](crate::KeyringStore); the OS may ask the user to allow
/// access). Only call this when the user asked for the import.
///
/// The entry is looked up by service and account first. The IDE stores an
/// account only when the caller passes a user name; database passwords have
/// none, and the IDE itself finds them by service alone, so that is the
/// fallback.
pub fn read_os_store(
    store: &dyn SecretStore,
    service_name: &str,
    account: Option<&str>,
) -> Result<Option<Secret>> {
    if let Some(account) = account.filter(|a| !a.is_empty())
        && let Some(secret) = store.get(service_name, account)?
    {
        return Ok(Some(secret));
    }
    store.get_any_account(service_name)
}

#[cfg(test)]
pub(crate) mod tests {
    use cbc::cipher::BlockModeEncrypt;

    use super::*;
    use crate::MemoryStore;
    use crate::keepass::{KeePassKey, KeePassStore};

    /// Encrypt like the IDE's built-in encryption (test helper).
    pub(crate) fn encrypt_built_in(plain: &[u8], iv: [u8; 16]) -> Vec<u8> {
        let body = cbc::Encryptor::<aes::Aes128>::new_from_slices(&BUILT_IN_KEY, &iv)
            .unwrap()
            .encrypt_padded_vec::<Pkcs7>(plain);
        let mut out = (iv.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&iv);
        out.extend_from_slice(&body);
        out
    }

    /// A `c.pwd` as the IDE writes it (test helper).
    pub(crate) fn main_key_file(encryption: &str, master: &str) -> String {
        let value = base64::engine::general_purpose::STANDARD
            .encode(encrypt_built_in(master.as_bytes(), [7u8; 16]));
        format!("encryption: {encryption}\nisAutoGenerated: true\nvalue: !!binary {value}")
    }

    #[test]
    fn service_names() {
        assert_eq!(
            generate_service_name("DB", "0d7c"),
            "IntelliJ Platform DB \u{2014} 0d7c"
        );
    }

    #[test]
    fn decodes_built_in_main_key() {
        let master = "c3ludGhldGljLW1hc3Rlci1rZXk";
        let file = main_key_file("BUILT_IN", master);
        let key = decode_main_key(file.as_bytes(), Platform::Linux).unwrap();
        assert_eq!(key.expose(), master);
        assert_eq!(
            parse_main_key_file(file.as_bytes(), Platform::MacOs)
                .unwrap()
                .encryption,
            MainKeyEncryption::BuiltIn
        );
        assert!(
            !format!(
                "{:?}",
                parse_main_key_file(file.as_bytes(), Platform::MacOs)
            )
            .contains(master)
        );

        // Lower case type, value wrapped over several lines.
        let value = base64::engine::general_purpose::STANDARD
            .encode(encrypt_built_in("pw".as_bytes(), [1u8; 16]));
        let (a, b) = value.split_at(10);
        let wrapped = format!("encryption: built_in\nvalue: !!binary |\n  {a}\n  {b}\n");
        assert_eq!(
            decode_main_key(wrapped.as_bytes(), Platform::Linux)
                .unwrap()
                .expose(),
            "pw"
        );
        // No type: the platform default.
        let untyped = format!("value: !!binary {value}");
        assert_eq!(
            decode_main_key(untyped.as_bytes(), Platform::MacOs)
                .unwrap()
                .expose(),
            "pw"
        );
    }

    #[test]
    fn decodes_old_main_key_file() {
        let raw = encrypt_built_in(b"old-master", [3u8; 16]);
        assert_eq!(
            decode_old_main_key(&raw, Platform::MacOs).unwrap().expose(),
            "old-master"
        );
    }

    #[test]
    fn main_key_errors() {
        let e = |text: &str| decode_main_key(text.as_bytes(), Platform::Linux).unwrap_err();
        assert_eq!(e("encryption: BUILT_IN\n").kind, SecretErrorKind::Format);
        assert_eq!(
            e("encryption: ROT13\nvalue: AAAA").kind,
            SecretErrorKind::Format
        );
        assert_eq!(
            e("encryption: BUILT_IN\nvalue: %%%").kind,
            SecretErrorKind::Format
        );
        assert_eq!(
            e("encryption: BUILT_IN\nvalue: !!binary AAAAEAAAAAAAAAAAAAAAAAAAAAA=").kind,
            SecretErrorKind::Format
        );
        let pgp = main_key_file("PGP_KEY", "x");
        assert_eq!(e(&pgp).kind, SecretErrorKind::Unavailable);
        #[cfg(not(windows))]
        {
            let crypt = main_key_file("CRYPT_32", "x");
            assert_eq!(e(&crypt).kind, SecretErrorKind::Unavailable);
        }
    }

    #[test]
    fn reads_main_key_from_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_main_key(dir.path()).unwrap().is_none());
        if Platform::current() != Platform::Windows {
            let old = encrypt_built_in(b"from-old-file", [5u8; 16]);
            std::fs::write(dir.path().join(OLD_MAIN_KEY_FILE_NAME), old).unwrap();
            assert_eq!(
                read_main_key(dir.path()).unwrap().unwrap().expose(),
                "from-old-file"
            );
            std::fs::write(
                dir.path().join(MAIN_KEY_FILE_NAME),
                main_key_file("BUILT_IN", "from-new-file"),
            )
            .unwrap();
            assert_eq!(
                read_main_key(dir.path()).unwrap().unwrap().expose(),
                "from-new-file"
            );
        }
    }

    #[test]
    fn settings_and_store_resolution() {
        let xml = r#"<application>
  <component name="Other"><option name="PROVIDER" value="MEMORY_ONLY" /></component>
  <component name="PasswordSafe">
    <option name="PROVIDER" value="KEEPASS" />
    <option name="keepassDb" value="$USER_HOME$/vault/ide.kdbx" />
  </component>
</application>"#;
        let s = parse_password_safe_settings(xml).unwrap();
        assert_eq!(s.provider, Some(ProviderType::KeePass));
        let cfg = Path::new("/cfg/PyCharm2025.2");
        let home = dirs::home_dir().unwrap();
        match resolve_store(cfg, &s, Platform::Linux) {
            IdeStore::KeePass {
                db_file,
                main_key_file,
            } => {
                assert_eq!(
                    db_file,
                    PathBuf::from(format!(
                        "{}/vault/ide.kdbx",
                        home.to_string_lossy().replace('\\', "/")
                    ))
                );
                assert_eq!(main_key_file, cfg.join("c.pwd"));
            }
            other => panic!("{other:?}"),
        }

        let defaults = PasswordSafeSettings::default();
        assert_eq!(
            resolve_store(cfg, &defaults, Platform::MacOs),
            IdeStore::OsStore
        );
        assert_eq!(
            resolve_store(cfg, &defaults, Platform::Windows),
            IdeStore::KeePass {
                db_file: cfg.join("c.kdbx"),
                main_key_file: cfg.join("c.pwd")
            }
        );
        let keychain = parse_password_safe_settings(
            r#"<application><component name="PasswordSafe"><option name="PROVIDER" value="KEYCHAIN"/></component></application>"#,
        )
        .unwrap();
        assert_eq!(
            resolve_store(cfg, &keychain, Platform::Linux),
            IdeStore::OsStore
        );
        assert!(matches!(
            resolve_store(cfg, &keychain, Platform::Windows),
            IdeStore::KeePass { .. }
        ));
        let memory = parse_password_safe_settings(
            r#"<application><component name="PasswordSafe"><option name="PROVIDER" value="DO_NOT_STORE"/></component></application>"#,
        )
        .unwrap();
        assert_eq!(
            resolve_store(cfg, &memory, Platform::MacOs),
            IdeStore::MemoryOnly
        );
        let config_macro = PasswordSafeSettings {
            provider: Some(ProviderType::KeePass),
            keepass_db: Some("$APPLICATION_CONFIG_DIR$/custom.kdbx".into()),
        };
        assert!(matches!(
            resolve_store(cfg, &config_macro, Platform::MacOs),
            IdeStore::KeePass { db_file, .. } if db_file == Path::new("/cfg/PyCharm2025.2/custom.kdbx")
        ));
        assert!(parse_password_safe_settings("<application></component>").is_err());
    }

    #[test]
    fn detects_store_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = detect_store(dir.path()).unwrap();
        match Platform::current() {
            Platform::Windows => assert!(matches!(store, IdeStore::KeePass { .. })),
            _ => assert_eq!(store, IdeStore::OsStore),
        }
        std::fs::create_dir_all(dir.path().join("options")).unwrap();
        std::fs::write(
            dir.path().join(SETTINGS_FILE),
            r#"<application><component name="PasswordSafe"><option name="PROVIDER" value="KEEPASS"/></component></application>"#,
        )
        .unwrap();
        assert_eq!(
            detect_store(dir.path()).unwrap(),
            IdeStore::KeePass {
                db_file: dir.path().join("c.kdbx"),
                main_key_file: dir.path().join("c.pwd"),
            }
        );
    }

    #[test]
    fn finds_config_dirs_newest_first() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "PyCharm2024.3",
            "PyCharmCE2025.1",
            "PyCharm2025.1",
            "PyCharm2025.10",
            "PyCharm2025.2",
            "DataGrip2025.2",
            "PyCharmCEnotes",
            "PyCharm",
        ] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::write(root.path().join("PyCharm2026.1"), b"a file").unwrap();
        let found: Vec<String> = find_config_dirs(root.path(), PYCHARM_PRODUCTS)
            .into_iter()
            .map(|d| format!("{}{}", d.product, d.version))
            .collect();
        assert_eq!(
            found,
            [
                "PyCharm2025.10",
                "PyCharm2025.2",
                "PyCharm2025.1",
                "PyCharmCE2025.1",
                "PyCharm2024.3"
            ]
        );
        assert!(find_config_dirs(&root.path().join("missing"), PYCHARM_PRODUCTS).is_empty());
    }

    fn ide_entries() -> Vec<(String, &'static str, &'static str)> {
        vec![
            (generate_service_name("DB", "uuid-1"), "app_user", "pw-1"),
            (generate_service_name("DB", "uuid-2"), "other", "pw-2"),
            (generate_service_name("DB", "uuid-2"), "report", "pw-2b"),
            (generate_service_name("Git", "example.com"), "", "tok"),
        ]
    }

    /// The IDE layout in KDBX 3.1, the format the IDE writes.
    pub(crate) fn ide_style_kdbx3(dir: &Path, master: &str) -> PathBuf {
        let entries = ide_entries();
        let entries: Vec<_> = entries
            .iter()
            .map(|(t, u, p)| (t.as_str(), *u, *p))
            .collect();
        let path = dir.join("c.kdbx");
        let data = crate::test_kdbx3::write(
            master,
            &[
                ("Other group", &[("unrelated", "x", "y")]),
                (SERVICE_NAME_PREFIX, &entries),
            ],
        );
        std::fs::write(&path, data).unwrap();
        path
    }

    /// The IDE layout in KDBX 4 (a database converted by another tool).
    fn ide_style_kdbx4(dir: &Path, master: &str) -> PathBuf {
        let path = dir.join("c4.kdbx");
        let store = KeePassStore::open(
            &path,
            KeePassKey::new(Secret::new(master)),
            crate::keepass::tests::fast_create(),
        )
        .unwrap();
        drop(store);
        let key = || DatabaseKey::new().with_password(master);
        let mut db = Database::parse(&std::fs::read(&path).unwrap(), key()).unwrap();
        {
            let mut root = db.root_mut();
            let mut group = root.add_group();
            group.name = SERVICE_NAME_PREFIX.to_owned();
            for (title, user, pw) in ide_entries() {
                let mut e = group.add_entry();
                e.set_unprotected(keepass::db::fields::TITLE, title);
                e.set_unprotected(keepass::db::fields::USERNAME, user);
                e.set_protected(keepass::db::fields::PASSWORD, pw);
            }
        }
        let mut out = Vec::new();
        db.save(&mut out, key()).unwrap();
        std::fs::write(&path, out).unwrap();
        path
    }

    #[test]
    fn reads_ide_keepass_kdbx3() {
        let dir = tempfile::tempdir().unwrap();
        check_ide_keepass(&ide_style_kdbx3(dir.path(), "auto-generated-master"));
    }

    #[test]
    fn reads_ide_keepass_kdbx4() {
        let dir = tempfile::tempdir().unwrap();
        check_ide_keepass(&ide_style_kdbx4(dir.path(), "auto-generated-master"));
    }

    fn check_ide_keepass(path: &Path) {
        let dir = path.parent().unwrap();
        let e = IdeKeePass::open(path, &Secret::new("wrong")).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Locked);
        assert_eq!(e.store, STORE_NAME);
        let e = IdeKeePass::open(&dir.join("none.kdbx"), &Secret::new("x")).unwrap_err();
        assert_eq!(e.kind, SecretErrorKind::Unavailable);

        let ide = IdeKeePass::open(path, &Secret::new("auto-generated-master")).unwrap();
        assert_eq!(ide.len(), 4);
        let s1 = generate_service_name("DB", "uuid-1");
        let c = ide.get(&s1, Some("app_user")).unwrap();
        assert_eq!(c.user.as_deref(), Some("app_user"));
        assert_eq!(c.password.unwrap().expose(), "pw-1");
        // A different user name still finds the only entry of the service.
        assert_eq!(
            ide.get(&s1, Some("renamed"))
                .unwrap()
                .password
                .unwrap()
                .expose(),
            "pw-1"
        );
        assert_eq!(
            ide.get(&s1, None).unwrap().password.unwrap().expose(),
            "pw-1"
        );
        // Two entries: the user name decides; no guess without a match.
        let s2 = generate_service_name("DB", "uuid-2");
        assert_eq!(
            ide.get(&s2, Some("report"))
                .unwrap()
                .password
                .unwrap()
                .expose(),
            "pw-2b"
        );
        assert!(ide.get(&s2, Some("nobody")).is_none());
        assert!(
            ide.get(&generate_service_name("DB", "uuid-3"), None)
                .is_none()
        );

        assert_eq!(
            ide.titles_with_prefix("IntelliJ Platform DB"),
            vec![s1.clone(), s2.clone()]
        );
        assert_eq!(ide.titles_with_prefix("IntelliJ Platform").len(), 3);
    }

    #[test]
    fn os_store_lookup() {
        let store = MemoryStore::default();
        let service = generate_service_name("DB", "uuid-1");
        store.set(&service, "app_user", &Secret::new("pw")).unwrap();
        assert_eq!(
            read_os_store(&store, &service, Some("app_user"))
                .unwrap()
                .unwrap()
                .expose(),
            "pw"
        );
        // A different user name still finds the entry by service alone.
        assert_eq!(
            read_os_store(&store, &service, Some("other"))
                .unwrap()
                .unwrap()
                .expose(),
            "pw"
        );
        // Entries the IDE saved without an account (database passwords).
        let no_account = generate_service_name("DB", "uuid-2");
        store.set(&no_account, "", &Secret::new("pw2")).unwrap();
        assert_eq!(
            read_os_store(&store, &no_account, None)
                .unwrap()
                .unwrap()
                .expose(),
            "pw2"
        );
        assert!(
            read_os_store(
                &store,
                &generate_service_name("DB", "nope"),
                Some("app_user")
            )
            .unwrap()
            .is_none()
        );
        assert!(os_store_support(Platform::MacOs).is_ok());
        assert!(os_store_support(Platform::Linux).is_err());
        assert!(os_store_support(Platform::Windows).is_err());
    }
}
