- crates/platform/secrets/src/intellij.rs, crates/platform/settings-import (README)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ fdafcf9ebe91959acb3d81110fc13ff490cf91ac
  Files: platform/credential-store-impl/src/credentialStore/EncryptionSupport.kt,
  keePass/mainKey.kt, keePass/KeePassCredentialStore.kt,
  keePass/BaseKeePassCredentialStore.kt, kdbx/KdbxGroup.kt,
  windows/WindowsCryptUtils.java, PasswordSafeSettings.kt,
  macOsKeychainLibrary.kt, linuxSecretLibrary.kt;
  platform/credential-store/src/credentialStore/CredentialAttributes.kt,
  CredentialStoreManagerImpl.kt, ProviderType.kt;
  jps/model-serialization/src/org/jetbrains/jps/model/serialization/PathMacroUtil.java.
  Adapted: the master key file format and its built-in AES key, the DPAPI
  wrapping on Windows, the settings file and provider names, the KeePass
  group and entry lookup, service name generation and path macro names.
  Changes: reimplemented in Rust, read-only, PGP-protected keys not
  supported; a single entry with a matching title is used when no entry
  matches the user name.
