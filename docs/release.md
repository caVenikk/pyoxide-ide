# Releases

How a release is cut, what it contains and how users install it. The decision
behind it (no Apple notarization, a formula instead of a cask) is
[ADR 0023](adr/0023-release-distribution-without-notarization.md).

## What a release contains

A GitHub Release `v<version>` with these assets (`<slug>` is the brand's
`SLUG`, today `pyoxide`):

| Asset | Contents |
|---|---|
| `<slug>-<version>-macos-aarch64.tar.gz`, `...-macos-x86_64.tar.gz` | `Pyoxide.app`: the three programs in `Contents/MacOS`, the debugger helper in `Contents/Resources/helpers/debugpy`, notices |
| `<slug>-<version>-linux-x86_64.tar.gz`, `...-linux-aarch64.tar.gz` | one folder: the three programs, `helpers/debugpy`, notices |
| `<slug>-<version>-windows-x86_64.zip` | one folder: the three programs (`.exe`), `conpty.dll`, `OpenConsole.exe`, `helpers/debugpy`, notices |
| `SHA256SUMS` | `<hash>  <file>` for every archive |
| `install.sh` | the install script for macOS and Linux |
| `<slug>.rb` | the Homebrew formula for this release |

The three programs are the IDE, the command line and the database command line
(`brand::IDE_NAME`, `CLI_NAME`, `DB_CLI_NAME`). `NOTICE.txt` is the complete
third-party text (`scripts/notice.py --out`), the debugger helper keeps its own
`ThirdPartyNotices.txt`.

## Cutting a release

1. Update the version in `[workspace.package]` of the root `Cargo.toml`, merge it.
2. Wait for CI on that commit of `main` to pass on all three platforms. The
   release workflow refuses a commit without a green `check (...)` job per OS.
3. Tag and push: `git tag v<version> && git push origin v<version>`. A version
   with a suffix (`v0.2.0-rc1`) becomes a pre-release.
4. The workflow `.github/workflows/release.yml` runs:
   - **preflight**: the tag is the workspace version, CI is green on the commit,
     the feature files, layers, brand sweep, notices and packaging names pass;
     the feature report is in the job summary (judge "MVP done" from it);
   - **build**: per platform `cargo build --release --locked`,
     `scripts/package.py build`, then `scripts/package.py verify` (unpack
     elsewhere, check files and signature, start the command line);
   - **release**: `SHA256SUMS`, the formula, release notes (install section and
     the generated change list) and a **draft** release.
5. Open the draft, read it, install from it on a machine (below), publish it.
   `install.sh` and the latest-release lookup only see a published release.
6. Update the Homebrew tap (below).

A manual run of the workflow (Actions > Release > Run workflow) builds the
archives and keeps them as workflow artifacts for 14 days; it creates no
release. Use it to try packaging changes.

Only the workflow's `GITHUB_TOKEN` is used; there are no secrets.

## How users install

### macOS and Linux: install script

```sh
curl -fsSL https://github.com/caVenikk/pyoxide-ide/releases/latest/download/install.sh | sh
```

`--version X.Y.Z` picks a version (needed for pre-releases), `--prefix DIR` the
install folder (macOS default `~/Applications`, Linux `~/.local/share`),
`--bin-dir DIR` the folder for the command-line launchers (default
`~/.local/bin`), `--uninstall` removes what it installed. The script downloads
the archive and `SHA256SUMS`, **stops if the SHA-256 does not match**, installs
in the user's home folder (no administrator rights) and removes the quarantine
attribute if the app has one. It is short; read it before running it.

### macOS and Linux: Homebrew

```sh
brew install caVenikk/tap/pyoxide
```

On macOS the app is in `$(brew --prefix)/opt/pyoxide`; the caveats show the
command that links it into `~/Applications`.

### Windows

Download the `.zip`, unpack it anywhere and run `pyoxide-ide.exe`. There is
no installer yet and no portable mode that keeps settings next to the program:
settings go to the user's configuration folder as usual.

### By hand

Download the archive for your system and `SHA256SUMS`, then
`shasum -a 256 -c --ignore-missing SHA256SUMS` (Linux: `sha256sum -c --ignore-missing`;
Windows: `Get-FileHash -Algorithm SHA256 <file>` and compare), unpack.

## What users see without notarization or a certificate

- **macOS.** The app is not notarized by Apple. Files downloaded by `curl`,
  by the install script or by Homebrew carry no quarantine attribute and open
  normally. An archive downloaded in a browser is quarantined: macOS says the
  app "cannot be opened" or "cannot be verified". Remove the attribute after
  checking the SHA-256:
  `xattr -dr com.apple.quarantine /path/to/Pyoxide.app`; or open System
  Settings > Privacy & Security and choose "Open Anyway" after the first
  attempt. The install script does the first for you, and says so.
- **Windows.** The executables are not signed. SmartScreen may show "Windows
  protected your PC": "More info" > "Run anyway". A file unpacked from a
  downloaded zip carries the "mark of the web"; "Unblock" in the zip's
  properties before unpacking avoids the prompt per file. Managed machines may
  block programs in user folders (AppLocker, Windows Defender Application
  Control); ask the administrator to allow the folder, or unpack to a location
  the policy allows.
- **Linux.** Nothing is signed; the checksum is the integrity check. The
  binaries need the system libraries of a desktop (X11 or Wayland, xkbcommon,
  fontconfig, Vulkan loader) and a glibc as old as Ubuntu 22.04's.

## Signing on macOS

`scripts/package.py` signs the programs and the bundle ad hoc, with a fixed
identifier per program and the designated requirement `identifier "<name>"`.
The requirement is the same in every build, so a keychain item that a user
allowed once stays allowed after an update
([code-signing.md](code-signing.md) explains access lists). It is weaker than
the certificate-based requirement of a local build signed by
`scripts/sign-local.sh`: any program that is ad hoc signed with the same
identifier satisfies it.

To build a macOS release on your own machine with the stable self-signed
identity of `scripts/sign-local.sh --setup`:

```sh
cargo build --release --locked -p pyoxide-ide -p pyoxide-cli -p db-cli
python3 scripts/package.py build --out dist \
  --sign-identity "Pyoxide Local Dev" --sign-keychain ~/Library/Keychains/pyoxide-signing.keychain-db
```

Such an archive can be attached to the draft release instead of the CI one.
Do not mix the two kinds in one series of releases: the requirement of an item
changes with the signature, and users would be prompted again once.

Neither kind is trusted by Gatekeeper; see above.

## Trying the install script locally

```sh
python3 scripts/package.py build --out dist          # needs target/release
cat dist/*.sha256 > dist/SHA256SUMS
PYOXIDE_BASE_URL=file://$PWD/dist sh packaging/install.sh \
  --version <version> --prefix /tmp/try/Applications --bin-dir /tmp/try/bin
```

`PYOXIDE_BASE_URL` replaces the release download folder; `file://` works.
`PYOXIDE_OS` and `PYOXIDE_ARCH` override detection. `scripts/package.py verify
<archive>` unpacks an archive elsewhere and starts it. Run `shellcheck
packaging/install.sh` when it is installed.

## The Homebrew tap

Homebrew finds `brew install <owner>/tap/<name>` in the GitHub repository
`<owner>/homebrew-tap`, folder `Formula/`. The formula is a formula and not a
cask on purpose: files a formula installs are not quarantined (ADR 0023).

### One-time setup (maintainer)

1. Create a public repository `homebrew-tap` under the same account as this
   repository (`caVenikk/homebrew-tap`), with a `README.md` and a folder
   `Formula/`.
2. Nothing else is needed: the workflow does not write to it (it has no token
   for other repositories).

### Per release

1. Download `pyoxide.rb` from the published release.
2. Put it into `Formula/pyoxide.rb` of the tap, commit, push.
3. Check: `brew update && brew install caVenikk/tap/pyoxide && pyoxide --version`,
   and `brew audit --strict --new caVenikk/tap/pyoxide`.

The template is `packaging/homebrew/formula.rb.in`; `scripts/package.py formula`
fills in the version, URLs and checksums from `SHA256SUMS`.

## Names

Packaging takes every name from `crates/python/brand/src/lib.rs`
(`scripts/package.py names`). `packaging/install.sh` is fetched with `curl` and
cannot read it, so it spells the names out in variables at its top;
`python3 scripts/package.py check` (part of `scripts/check.sh` and CI) fails
when they differ from the brand. A rebrand also touches the manual steps in
[rebrand.md](rebrand.md).

## Not done yet

- A Windows installer (per-user and per-machine) and a portable mode with a
  `data` folder; Windows on arm64.
- A `.dmg`, an AppImage, a `.deb`.
- An application icon (`.icns`, `.ico`).
- Notarization and signing with purchased certificates.
- Third-party license texts of the Rust dependencies in the archives (the
  notices and the application's own licenses are included).
- An update check in the application.
