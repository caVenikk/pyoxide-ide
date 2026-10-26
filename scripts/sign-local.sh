#!/usr/bin/env bash
# Signs local macOS builds with one stable self-signed code-signing identity.
#
# A binary without a signature, or signed ad hoc (what the linker does on
# Apple silicon), has a new code identity with every build. Keychain access
# lists match on that identity, so macOS asks for the login password again
# after each rebuild. A fixed identity and a fixed identifier give every build
# the same designated requirement; "Always Allow" once and the prompts stop.
#
#   scripts/sign-local.sh                     sign target/release/{pyoxide-ide,pyoxide-db,pyoxide}
#   scripts/sign-local.sh PATH...             sign these binaries (or .app bundles)
#   scripts/sign-local.sh --setup             print the one-time commands that create the identity
#   scripts/sign-local.sh --check PATH...     print the designated requirement of binaries
#
# Environment:
#   PYOXIDE_SIGN_IDENTITY   certificate common name (default: Pyoxide Local Dev)
#   PYOXIDE_SIGN_KEYCHAIN   keychain that holds it (default:
#                           ~/Library/Keychains/pyoxide-signing.keychain-db)
#
# The identity lives in its own keychain file, never in the login keychain.
# Nothing here reads or writes keychain items of the application.
# Release builds are signed with a Developer ID instead: docs/code-signing.md.
set -euo pipefail

IDENTITY="${PYOXIDE_SIGN_IDENTITY:-Pyoxide Local Dev}"
KEYCHAIN="${PYOXIDE_SIGN_KEYCHAIN:-$HOME/Library/Keychains/pyoxide-signing.keychain-db}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [ "$(uname -s)" != "Darwin" ]; then
    echo "sign-local.sh: macOS only" >&2
    exit 1
fi

setup() {
    cat <<EOF
One-time setup of the local signing identity "$IDENTITY".
Run these in a terminal; nothing touches the login keychain.

  KC="$KEYCHAIN"
  WORK="\$(mktemp -d)"
  security create-keychain -p "" "\$KC"
  security set-keychain-settings "\$KC"          # never auto-lock
  security unlock-keychain -p "" "\$KC"

  # Self-signed certificate valid for code signing, 10 years. The pkcs12
  # algorithms are the old ones: the macOS importer rejects OpenSSL 3's.
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \\
      -subj "/CN=$IDENTITY" \\
      -addext "keyUsage=critical,digitalSignature" \\
      -addext "extendedKeyUsage=critical,codeSigning" \\
      -keyout "\$WORK/key.pem" -out "\$WORK/cert.pem"
  openssl pkcs12 -export -inkey "\$WORK/key.pem" -in "\$WORK/cert.pem" \\
      -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \\
      -out "\$WORK/id.p12" -passout pass:pyoxide-local
  security import "\$WORK/id.p12" -k "\$KC" -P pyoxide-local -T /usr/bin/codesign
  security set-key-partition-list -S apple-tool:,apple: -s -k "" "\$KC" >/dev/null
  rm -rf "\$WORK"

  # Check:
  security find-identity -p codesigning "\$KC"

Then: scripts/sign-local.sh
EOF
}

identity_present() {
    [ -f "$KEYCHAIN" ] && security find-identity -p codesigning "$KEYCHAIN" 2>/dev/null | grep -q "\"$IDENTITY\""
}

identifier_for() {
    # A fixed identifier per binary. Without one, codesign derives it from
    # the file name plus a hash of the contents, which changes per build.
    basename "${1%.app}"
}

sign_one() {
    local path="$1"
    if [ ! -e "$path" ]; then
        echo "sign-local.sh: skip $path (not built)" >&2
        return 0
    fi
    security unlock-keychain -p "" "$KEYCHAIN" 2>/dev/null || true
    codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" \
        --identifier "$(identifier_for "$path")" "$path"
    echo "signed $path"
    codesign --display --requirements - "$path" 2>&1 | grep -E '^designated' || true
}

mode=sign
case "${1:-}" in
    --setup)
        setup
        exit 0
        ;;
    --check)
        mode=check
        shift
        ;;
    -h | --help)
        sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
        exit 0
        ;;
esac

if [ "$mode" = check ]; then
    for path in "$@"; do
        echo "$path"
        codesign --display --requirements - "$path" 2>&1 | grep -E '^(designated|Executable)' || echo "  not signed"
    done
    exit 0
fi

if ! identity_present; then
    echo "sign-local.sh: no code-signing identity \"$IDENTITY\" in $KEYCHAIN" >&2
    echo "Create it once with:  scripts/sign-local.sh --setup" >&2
    exit 1
fi

if [ "$#" -eq 0 ]; then
    set -- "$ROOT"/target/release/pyoxide-ide "$ROOT"/target/release/pyoxide-db "$ROOT"/target/release/pyoxide
fi
for path in "$@"; do
    sign_one "$path"
done
