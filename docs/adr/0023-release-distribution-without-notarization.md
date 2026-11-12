# 0023. Release distribution without Apple notarization

- Status: accepted
- Date: 2026-10-28

## Context
Releases must be installable by people who did not build the project (issue
#94). The project has no paid Apple Developer ID and no Windows code-signing
certificate, so macOS builds cannot be notarized and Windows builds trigger
SmartScreen. Constraints:

- On macOS, Gatekeeper refuses an app that is not notarized when the file
  carries the quarantine attribute (`com.apple.quarantine`). Browsers, and
  casks installed through Homebrew, set the attribute; `curl`, `tar` and a
  Homebrew formula's own files do not. Apple silicon requires at least an ad
  hoc signature.
- A keychain item (data source passwords, ADR 0004) is readable without a
  prompt only by programs in its access list, which names programs by code
  requirement. The default requirement of an ad hoc signature is the hash of
  that build's code, so every update would be a stranger to the items
  (docs/code-signing.md).
- The release workflow has the repository's `GITHUB_TOKEN` and nothing else:
  no signing keys, no tokens for other repositories.

## Options considered
1. **Cask in the official Homebrew repository.** Needs a notarized app; a cask
   that fails Gatekeeper is removed. Not possible.
2. **Cask in our own tap.** Homebrew quarantines cask downloads, so the first
   start would need System Settings > Privacy & Security > Open Anyway. Not
   acceptable as the main path.
3. **Formula in our own tap, plus an install script.** A formula's download is
   not quarantined; neither is a file fetched by `curl`. The app opens
   normally. Costs: a separate tap repository to maintain, and an app installed
   by a formula lives in the Cellar, linked by hand into Applications.
4. **Disk image (`.dmg`) or zip downloaded in a browser.** Quarantined; needs
   Open Anyway. Offered only as a manual download with the steps documented.
5. **Signing identity.** A self-signed certificate held as a repository secret
   would give a stable certificate-based requirement. It is not available with
   `GITHUB_TOKEN` only, and it would not satisfy Gatekeeper anyway.

## Decision
- Archives are produced by `scripts/package.py` on CI runners for macOS
  (arm64, x86_64), Linux (x86_64, arm64) and Windows (x86_64) and attached to a
  draft GitHub Release with a `SHA256SUMS` file. macOS: `.tar.gz` holding the
  `.app`; Linux: `.tar.gz`; Windows: `.zip`.
- macOS install paths: `packaging/install.sh` (`curl | sh`: checks the
  SHA-256, installs under `~/Applications`, removes the quarantine attribute
  with an explanation, no administrator rights) and a **formula** in a tap
  repository (`packaging/homebrew/`, rendered per release and attached to it;
  the maintainer copies it into the tap). Linux uses the script or the formula
  too. Windows: unpack the zip.
- macOS binaries are signed ad hoc with a fixed identifier and an explicit
  designated requirement `identifier "<name>"`, the same in every build, so
  keychain access lists carry over between updates. This is weaker than a
  certificate-based requirement: a program that is ad hoc signed with the same
  identifier satisfies it. It matches what a local development build gets
  (docs/code-signing.md names the stronger option). A maintainer who builds a
  release locally may pass a self-signed identity to `package.py` instead.
- No notarization, no Windows signing. The documentation says what users see
  (Gatekeeper, SmartScreen, AppLocker/WDAC) and how to continue.
- ConPTY files for Windows (`conpty.dll`, `OpenConsole.exe`, MIT) come from the
  pinned NuGet package Microsoft.Windows.Console.ConPTY, checked by SHA-512;
  the debugger helper from the pinned debugpy wheel (ADR 0015).

## Consequences
- A release is a tag; everything else is the workflow. Nothing is published
  automatically: the release is a draft, the tap is updated by hand.
- If a paid Developer ID becomes available: sign with it and notarize in
  `package.py`, offer a cask and a `.dmg`, keep the formula and the script. The
  designated requirement changes then, and users are prompted once
  (docs/code-signing.md).
- Updating means running the script or `brew upgrade` again; there is no
  update check in the application yet.
- Not covered yet: a Windows installer and a portable mode with a `data`
  folder, an AppImage or `.deb`, Windows arm64, an application icon.
