# Code signing and keychain prompts (macOS)

The OS credential store keeps data source passwords (service `pyoxide-db`).
On macOS a keychain item can be read without a prompt only by the code that
created it and by the applications in the item's access list. Any other
reader makes the system ask for the login password ("... wants to use
confidential information stored in pyoxide-db"). Three things decide how
often that happens.

## 1. How often the application reads

`secrets::CachedStore` wraps the OS store: a secret is read from the keychain
at most once per process. Connecting, reconnecting and keep-alive use the
copy in memory (wiped on drop). Writes through the same store keep the copy
current; a password changed by another process shows up after a restart.
Failures and missing entries are not remembered. The settings file
(`store = "keychain"`, the default) opens the store this way for the IDE and
the command line alike.

## 2. Who is in the access list

`KeyringStore::set` on macOS creates the item with an explicit access list
(`SecAccessCreate` and `SecKeychainItemCreateFromContent`, module
`secrets::macos_acl`) naming the running binary and its siblings
`pyoxide-ide`, `pyoxide-db` and `pyoxide` in the same directory. An item
stored by the CLI is then trusted by the IDE and the other way round, with no
prompt for either. Setting a secret again replaces the item, so its list is
refreshed with it. The `keyring` crate cannot do this: it uses the default
list (the creating binary only) and exposes no access control. The legacy
file-keychain API is the only one with per-application lists; the data
protection keychain would need a keychain access group and an entitlement
tied to a provisioning profile, which a command line tool does not have.

Reading never changes the list. An item created before this change, or by
another program, keeps its old list until it is stored again
(`pyoxide-db secret set <profile>`, or save the data source in the IDE).

PyCharm creates its items with the default list and is the only reader, so it
is not prompted once its signature is stable. Pyoxide has two binaries that
share items, hence the explicit list.

## 3. Whether the code identity is stable

The access list holds code requirements, not paths. For a binary signed ad
hoc (the linker does this on Apple silicon, so every plain `cargo build`
output is) the designated requirement is the hash of the code itself:

```
designated => cdhash H"4e77..." or cdhash H"792a..." or ...
```

Every build has another hash, so every build is a stranger to every item, and
"Always Allow" does not carry over. Signed with a fixed certificate and a
fixed identifier, the requirement does not depend on the build:

```
designated => identifier "pyoxide-ide" and certificate leaf = H"50d4..."
```

### Local builds

Sign every local build with one self-signed identity. It lives in a keychain
file of its own, never in the login keychain.

1. `scripts/sign-local.sh --setup` prints the one-time commands: they create
   `~/Library/Keychains/pyoxide-signing.keychain-db` and the certificate
   "Pyoxide Local Dev" in it. Run them once.
2. After each build: `scripts/sign-local.sh` (signs `target/release/pyoxide-ide`,
   `pyoxide-db` and `pyoxide`; pass other paths to sign those). A rebuild
   replaces the signature, so sign again after every build you run.
3. `scripts/sign-local.sh --check target/release/pyoxide-ide` shows the
   designated requirement; it must show `certificate leaf`, not `cdhash`.
4. Items created before the first signed build still list the old identities.
   Store them once more (`pyoxide-db secret set <profile>`, signed
   `pyoxide-db`; replacing the old item may ask for the login password once) or click "Always Allow" once per binary; both work from then
   on, whatever the build.

The certificate is not trusted by the system (`CSSMERR_TP_NOT_TRUSTED` in
`security find-identity`); that does not matter for keychain access lists,
only the leaf hash does. Gatekeeper still treats the binary as unsigned by a
developer, as for any local build.

### Releases

Without a paid Developer ID (the current state), releases are signed ad hoc
with a fixed identifier and an explicit designated requirement, or with a
local self-signed identity; see [release.md](release.md) and ADR 0023. The
rest of this section describes the signing a Developer ID would allow.

Release binaries would be signed with one Developer ID Application certificate,
all of them (`pyoxide-ide`, `pyoxide-db`, `pyoxide`, and the `.app` bundle
that holds them), with a fixed identifier each:

```bash
codesign --force --timestamp --options runtime \
  --sign "Developer ID Application: <Name> (<TEAMID>)" \
  --identifier pyoxide-ide --entitlements release.entitlements pyoxide-ide
```

- The same certificate for every binary: the access list of an item names
  each binary by its requirement, and recent macOS versions also check a
  partition list on the item (`teamid:<TEAMID>` for items created by code
  signed with a Team ID). Binaries of one team share it. Not yet checked on a
  Developer ID build: if an item created by one binary still prompts the
  other, `security set-generic-password-partition-list -S teamid:<TEAMID> -s pyoxide-db`
  fixes existing items (it asks for the login password once).
- The requirement survives updates: an item a user allowed once stays allowed
  in every later release signed with the same certificate and identifier.
  Items the application created itself need no prompt at all.
- The hardened runtime (`--options runtime`) is required for notarization. The
  Oracle client libraries are loaded at run time from the user's machine and
  are not signed by us, so the entitlement
  `com.apple.security.cs.disable-library-validation` goes into
  `release.entitlements`.
- Notarize with `xcrun notarytool submit --wait` and staple the ticket to the
  `.app`/disk image; plain command line binaries cannot be stapled, notarize
  the archive that holds them.
- Changing the certificate (a new team, a renewed ID with a new key) changes
  the requirement: users are prompted once again. Keep the key.

Where the signature can be checked: `codesign --verify --strict --verbose=2`,
`codesign --display --requirements - <binary>`, `spctl --assess --type exec`.

## Testing without touching real keychains

Unit tests use `MemoryStore` and counting wrappers. The macOS access list
test creates a scratch keychain file (`SecKeychainCreate`, outside the search
list) and removes it afterwards; it never opens the login keychain. To try
the real thing by hand, use a scratch keychain as well:

```bash
security create-keychain -p x /tmp/scratch.keychain-db   # unlocked, not in the search list
security dump-keychain -a /tmp/scratch.keychain-db       # lists each item's access list
security delete-keychain /tmp/scratch.keychain-db
```
