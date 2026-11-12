#!/usr/bin/env python3
# Release packaging (docs/release.md): turns the release binaries of this
# machine into the archive a user downloads, and renders the files that
# describe a release. Standard library only.
#
#     python3 scripts/package.py build --out dist              # the archive of this OS and CPU
#     python3 scripts/package.py formula --version V --sums SHA256SUMS --out pyoxide.rb
#     python3 scripts/package.py notes --version V --out notes.md
#     python3 scripts/package.py verify dist/<archive>          # unpack elsewhere, check, start it
#     python3 scripts/package.py names                     # the names packaging uses
#     python3 scripts/package.py check                          # packaging/ agrees with the brand
#
# `build` takes target/release (or --bin-dir) and writes, into --out:
#
#   macOS    <slug>-<version>-macos-<arch>.tar.gz     <Product>.app (binaries in Contents/MacOS,
#                                                      debugpy in Contents/Resources/helpers)
#   Linux    <slug>-<version>-linux-<arch>.tar.gz     one folder: binaries, helpers/debugpy
#   Windows  <slug>-<version>-windows-<arch>.zip      the same, .exe, conpty.dll, OpenConsole.exe
#
# and `<archive>.sha256` (`<hash>  <name>`, the format `sha256sum -c` reads).
#
# macOS binaries are signed here: ad hoc by default, with an explicit
# designated requirement that names only the identifier, so the requirement
# is the same in every build (a plain ad hoc signature names the hash of the
# code, which changes with every build, and macOS keychain access lists
# would ask again after each update; docs/code-signing.md). With
# --sign-identity and --sign-keychain (a local identity made by
# scripts/sign-local.sh --setup) the binaries are signed with that
# certificate instead, which gives the stronger requirement.
#
# Product names come from crates/python/brand/src/lib.rs, never from literals
# here. Exit code 1 on any problem.
import argparse
import hashlib
import platform
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / "crates" / "python" / "brand" / "src" / "lib.rs"
PACKAGING = ROOT / "packaging"
FORMULA_TEMPLATE = PACKAGING / "homebrew" / "formula.rb.in"
NOTES_TEMPLATE = PACKAGING / "release-notes.md.in"
INSTALL_SCRIPT = PACKAGING / "install.sh"
FONT_LICENSES = ROOT / "crates" / "platform" / "ui" / "assets" / "fonts"

# The CPUs a release is built for, by the names of the archives.
ARCHES = {"arm64": "aarch64", "aarch64": "aarch64", "amd64": "x86_64", "x86_64": "x86_64"}
# The macOS the bundle declares as its minimum (not tested on older systems).
MACOS_MINIMUM = "13.0"


def fail(message):
    print(f"package.py: {message}", file=sys.stderr)
    sys.exit(1)


def run(command, **kwargs):
    result = subprocess.run(command, text=True, **kwargs)
    if result.returncode != 0:
        fail(f"`{' '.join(str(c) for c in command)}` failed ({result.returncode})")
    return result


def python():
    return sys.executable


# ---------------------------------------------------------------- names


class Names:
    """What packaging calls things, read from the brand module."""

    def __init__(self):
        text = BRAND.read_text(encoding="utf-8")

        def const(name):
            found = re.search(rf'pub const {name}: &str = "([^"]+)";', text)
            if not found:
                fail(f"{name} not found in {BRAND.relative_to(ROOT)}")
            return found.group(1)

        slug = re.search(r"macro_rules! slug \{\s*\(\) => \{\s*\"([^\"]+)\"", text)
        if not slug:
            fail(f"the slug macro not found in {BRAND.relative_to(ROOT)}")
        self.slug = slug.group(1)
        self.product = const("PRODUCT_NAME")
        self.full_name = const("PRODUCT_FULL_NAME")
        self.bundle_id = const("BUNDLE_ID")
        # CLI_NAME = slug, DB_CLI_NAME = slug-db, IDE_NAME = slug-ide.
        self.cli = self.slug
        self.db = f"{self.slug}-db"
        self.ide = f"{self.slug}-ide"
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        version = re.search(r'^\[workspace\.package\][^\[]*?^version = "([^"]+)"', cargo, re.M | re.S)
        repository = re.search(r'^repository = "https://github\.com/([^"/]+/[^"/]+)"', cargo, re.M)
        if not version or not repository:
            fail("version or repository not found in the workspace Cargo.toml")
        self.version = version.group(1)
        self.repository = repository.group(1)

    @property
    def app(self):
        return f"{self.product}.app"

    @property
    def binaries(self):
        return [self.ide, self.db, self.cli]

    def table(self):
        return {
            "PRODUCT_NAME": self.product,
            "PRODUCT_FULL_NAME": self.full_name,
            "SLUG": self.slug,
            "CLASS": "".join(part.capitalize() for part in re.split(r"[-_]", self.slug)),
            "BUNDLE_ID": self.bundle_id,
            "CLI": self.cli,
            "DB_CLI": self.db,
            "IDE": self.ide,
            "APP": self.app,
            "REPOSITORY": self.repository,
            "REPOSITORY_OWNER": self.repository.split("/")[0],
        }


def host_os():
    if sys.platform == "darwin":
        return "macos"
    if sys.platform == "win32":
        return "windows"
    if sys.platform.startswith("linux"):
        return "linux"
    fail(f"no release for {sys.platform}")


def host_arch(override):
    machine = (override or platform.machine()).lower()
    if machine not in ARCHES:
        fail(f"no release for the CPU {machine!r}")
    return ARCHES[machine]


def archive_name(names, version, os_name, arch):
    suffix = "zip" if os_name == "windows" else "tar.gz"
    return f"{names.slug}-{version}-{os_name}-{arch}.{suffix}"


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


# ---------------------------------------------------------------- build


def copy_binaries(names, bin_dir, dest, os_name):
    suffix = ".exe" if os_name == "windows" else ""
    dest.mkdir(parents=True, exist_ok=True)
    for binary in names.binaries:
        source = bin_dir / f"{binary}{suffix}"
        if not source.is_file():
            fail(f"{source} is missing; build with `cargo build --release` first")
        shutil.copy2(source, dest / source.name)


def put_debugpy(dest, given):
    """The debugger helper (ADR 0015), unmodified: copied from a folder that
    holds one, else fetched and checked by scripts/fetch-debugpy.py."""
    if given:
        for marker in ("debugpy/adapter/__main__.py",):
            if not (given / marker).is_file():
                fail(f"{given} is not a debugpy helper folder ({marker} is missing)")
        # A folder that was used has bytecode in it, which is not the wheel's.
        shutil.copytree(given, dest, ignore=shutil.ignore_patterns("__pycache__"))
    else:
        run([python(), str(ROOT / "scripts" / "fetch-debugpy.py"), "--dest", str(dest)])
    if not (dest / "debugpy" / "ThirdPartyNotices.txt").is_file():
        fail("the debugpy helper has no debugpy/ThirdPartyNotices.txt; its notices must ship")


def put_notices(folder):
    """NOTICE (the complete third-party text), the licences, the fonts' licences."""
    folder.mkdir(parents=True, exist_ok=True)
    run([python(), str(ROOT / "scripts" / "notice.py"), "--out", str(folder / "NOTICE.txt")],
        stdout=subprocess.DEVNULL)
    for name in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy2(ROOT / name, folder / name)
    licenses = folder / "licenses"
    licenses.mkdir(exist_ok=True)
    for license_file in sorted(FONT_LICENSES.glob("LICENSE-*.txt")):
        shutil.copy2(license_file, licenses / license_file.name)


def put_conpty(dest, given, arch):
    """conpty.dll and OpenConsole.exe next to the executable (ADR 0014)."""
    if arch != "x86_64":
        fail("ConPTY files are pinned for x86_64 only")
    if given:
        for name in ("conpty.dll", "OpenConsole.exe"):
            if not (given / name).is_file():
                fail(f"{given / name} is missing")
            shutil.copy2(given / name, dest / name)
    else:
        run([python(), str(ROOT / "scripts" / "fetch-conpty.py"), "--dest", str(dest)])


def codesign(path, identifier, identity, keychain):
    command = ["codesign", "--force", "--sign", identity, "--identifier", identifier]
    if identity == "-":
        # Ad hoc: the default requirement is the hash of this build's code.
        command += [f"-r=designated => identifier \"{identifier}\""]
    elif keychain:
        command += ["--keychain", str(keychain)]
    run(command + [str(path)], stderr=subprocess.DEVNULL)


def sign_bundle(names, app, identity, keychain):
    macos = app / "Contents" / "MacOS"
    # Every binary on its own, with a fixed identifier (the name of the
    # program, which keychain access lists name), then the bundle.
    for binary in (names.db, names.cli):
        codesign(macos / binary, binary, identity, keychain)
    codesign(app, names.ide, identity, keychain)
    run(["codesign", "--verify", "--strict", "--deep", str(app)])
    for target in (app, macos / names.db, macos / names.cli):
        shown = subprocess.run(["codesign", "--display", "-r-", str(target)],
                               capture_output=True, text=True)
        requirement = next((line for line in (shown.stdout + shown.stderr).splitlines()
                            if line.startswith("designated")), "?")
        print(f"signed {target.relative_to(app.parent)}: {requirement}")


def make_app(names, version, bin_dir, root, args):
    app = root / names.app
    contents = app / "Contents"
    macos = contents / "MacOS"
    copy_binaries(names, bin_dir, macos, "macos")
    resources = contents / "Resources"
    put_debugpy(resources / "helpers" / "debugpy", args.debugpy)
    put_notices(resources)
    info = {
        "CFBundleName": names.product,
        "CFBundleDisplayName": names.product,
        "CFBundleIdentifier": names.bundle_id,
        "CFBundleExecutable": names.ide,
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": version,
        "CFBundleShortVersionString": version,
        "LSMinimumSystemVersion": MACOS_MINIMUM,
        "NSHighResolutionCapable": True,
        "LSApplicationCategoryType": "public.app-category.developer-tools",
    }
    with open(contents / "Info.plist", "wb") as fh:
        plistlib.dump(info, fh)
    sign_bundle(names, app, args.sign_identity, args.sign_keychain)
    return app


def make_folder(names, bin_dir, root, os_name, arch, args):
    copy_binaries(names, bin_dir, root, os_name)
    put_debugpy(root / "helpers" / "debugpy", args.debugpy)
    put_notices(root)
    if os_name == "windows":
        put_conpty(root, args.conpty, arch)
    return root


def tar_filter(info):
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    return info


def write_archive(root, top_name, archive):
    """The folder `root` as `archive`, its entries under `top_name`."""
    entries = sorted(root.rglob("*"))
    if archive.name.endswith(".zip"):
        with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
            for path in entries:
                if path.is_file():
                    zf.write(path, f"{top_name}/{path.relative_to(root).as_posix()}")
        return
    with tarfile.open(archive, "w:gz") as tf:
        tf.add(root, arcname=top_name, recursive=False, filter=tar_filter)
        for path in entries:
            tf.add(path, arcname=f"{top_name}/{path.relative_to(root).as_posix()}",
                   recursive=False, filter=tar_filter)


def cmd_build(args):
    names = Names()
    version = args.version or names.version
    os_name = host_os()
    arch = host_arch(args.arch)
    bin_dir = (args.bin_dir or ROOT / "target" / "release").resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    archive = out / archive_name(names, version, os_name, arch)
    if os_name != "macos" and args.sign_identity != "-":
        fail("--sign-identity is for macOS builds")
    with tempfile.TemporaryDirectory(prefix="package-") as tmp:
        stage = Path(tmp) / "stage"
        stage.mkdir()
        if os_name == "macos":
            app = make_app(names, version, bin_dir, stage, args)
            write_archive(app, names.app, archive)
        else:
            folder = stage / f"{names.slug}-{version}-{os_name}-{arch}"
            make_folder(names, bin_dir, folder, os_name, arch, args)
            write_archive(folder, folder.name, archive)
    digest = sha256_of(archive)
    (out / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
    print(f"{archive} ({archive.stat().st_size} bytes)\nsha256 {digest}")
    return 0


# ---------------------------------------------------------------- rendering


def render(template, table):
    text = template.read_text(encoding="utf-8")
    for key, value in table.items():
        text = text.replace(f"@{key}@", value)
    left = re.findall(r"@[A-Z0-9_]+@", text)
    if left:
        fail(f"{template.name}: no value for {sorted(set(left))}")
    return text


def read_sums(path):
    sums = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        found = re.fullmatch(r"([0-9a-f]{64}) [ *](\S+)", line.strip())
        if found:
            sums[found.group(2)] = found.group(1)
    return sums


def cmd_formula(args):
    names = Names()
    sums = read_sums(args.sums)
    table = names.table()
    table["VERSION"] = args.version
    for key, os_name, arch in (
        ("SHA_MACOS_AARCH64", "macos", "aarch64"),
        ("SHA_MACOS_X86_64", "macos", "x86_64"),
        ("SHA_LINUX_X86_64", "linux", "x86_64"),
        ("SHA_LINUX_AARCH64", "linux", "aarch64"),
    ):
        name = archive_name(names, args.version, os_name, arch)
        if name not in sums:
            fail(f"{args.sums} has no checksum for {name}")
        table[key] = sums[name]
    args.out.write_text(render(FORMULA_TEMPLATE, table), encoding="utf-8")
    print(f"{args.out}: formula for {args.version}")
    return 0


def cmd_notes(args):
    names = Names()
    table = names.table()
    table["VERSION"] = args.version
    table["TAG"] = f"v{args.version}"
    args.out.write_text(render(NOTES_TEMPLATE, table), encoding="utf-8")
    print(f"{args.out}: release notes header for {args.version}")
    return 0


def cmd_verify(args):
    """Unpacks an archive made by `build` somewhere else and tries it: the
    files a release must hold are there, the signature (macOS) holds after
    the round trip, and the command-line program starts."""
    names = Names()
    archive = args.archive.resolve()
    os_name = host_os()
    suffix = ".exe" if os_name == "windows" else ""
    with tempfile.TemporaryDirectory(prefix="verify-") as tmp:
        tmp = Path(tmp)
        if archive.suffix == ".zip":
            with zipfile.ZipFile(archive) as zf:
                zf.extractall(tmp)
        else:
            with tarfile.open(archive) as tf:
                tf.extractall(tmp, filter="data")
        tops = list(tmp.iterdir())
        if len(tops) != 1:
            fail(f"{archive.name}: expected one top-level entry, found {[t.name for t in tops]}")
        top = tops[0]
        if os_name == "macos":
            if top.name != names.app:
                fail(f"{archive.name}: top-level entry is {top.name}, expected {names.app}")
            programs, resources = top / "Contents" / "MacOS", top / "Contents" / "Resources"
            run(["codesign", "--verify", "--strict", "--deep", str(top)])
        else:
            programs = resources = top
        wanted = [programs / f"{b}{suffix}" for b in names.binaries]
        wanted += [resources / "helpers" / "debugpy" / "debugpy" / "adapter" / "__main__.py",
                   resources / "helpers" / "debugpy" / "debugpy" / "ThirdPartyNotices.txt",
                   resources / "NOTICE.txt", resources / "LICENSE-MIT", resources / "LICENSE-APACHE"]
        if os_name == "windows":
            wanted += [top / "conpty.dll", top / "OpenConsole.exe"]
        missing = [str(p.relative_to(top)) for p in wanted if not p.is_file()]
        if missing:
            fail(f"{archive.name}: missing {missing}")
        pycache = [p for p in resources.rglob("__pycache__")]
        if pycache:
            fail(f"{archive.name}: bytecode folders in the archive: {pycache[:3]}")
        shown = run([str(programs / f"{names.cli}{suffix}"), "--version"], capture_output=True)
        print(shown.stdout.strip())
        if (args.version or names.version) not in shown.stdout:
            fail(f"`{names.cli} --version` does not print {args.version or names.version}")
    print(f"{archive.name}: complete, starts")
    return 0


def cmd_names(_args):
    names = Names()
    for key, value in {**names.table(), "VERSION": names.version}.items():
        print(f"{key}={value}")
    return 0


def cmd_check(_args):
    """packaging/ spells names out in files that cannot read the brand
    module (a script fetched with curl, a formula); they must agree with it."""
    names = Names()
    problems = []
    script = INSTALL_SCRIPT.read_text(encoding="utf-8")
    wanted = {
        "PRODUCT": names.product,
        "SLUG": names.slug,
        "REPOSITORY": names.repository,
        "CLI": names.cli,
        "DB_CLI": names.db,
        "IDE": names.ide,
    }
    for key, value in wanted.items():
        found = re.search(rf'^{key}="([^"]*)"', script, re.M)
        if not found or found.group(1) != value:
            problems.append(f"packaging/install.sh: {key} is {found.group(1) if found else 'missing'}, "
                            f"the brand says {value}")
    for template in (FORMULA_TEMPLATE, NOTES_TEMPLATE):
        text = template.read_text(encoding="utf-8")
        for stray in {names.slug, names.product} & set(re.findall(r"[A-Za-z][A-Za-z-]*", text)):
            problems.append(f"{template.relative_to(ROOT)}: spells the name {stray!r}; use the @...@ values")
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print("packaging/: names agree with the brand module")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Release packaging.")
    sub = parser.add_subparsers(dest="command", required=True)

    build = sub.add_parser("build", help="package the release binaries of this machine")
    build.add_argument("--bin-dir", type=Path, help="where the binaries are (default: target/release)")
    build.add_argument("--out", type=Path, required=True, help="where the archive goes")
    build.add_argument("--version", help="default: the workspace version")
    build.add_argument("--arch", help="aarch64 or x86_64 (default: this machine's)")
    build.add_argument("--debugpy", type=Path, help="a debugpy helper folder to copy instead of fetching")
    build.add_argument("--conpty", type=Path, help="a folder with conpty.dll and OpenConsole.exe (Windows)")
    build.add_argument("--sign-identity", default="-", help="macOS: a code-signing identity (default: ad hoc)")
    build.add_argument("--sign-keychain", type=Path, help="macOS: the keychain that holds the identity")
    build.set_defaults(run=cmd_build)

    formula = sub.add_parser("formula", help="render the Homebrew formula for a release")
    formula.add_argument("--version", required=True)
    formula.add_argument("--sums", type=Path, required=True, help="SHA256SUMS of the release")
    formula.add_argument("--out", type=Path, required=True)
    formula.set_defaults(run=cmd_formula)

    notes = sub.add_parser("notes", help="render the install section of the release notes")
    notes.add_argument("--version", required=True)
    notes.add_argument("--out", type=Path, required=True)
    notes.set_defaults(run=cmd_notes)

    verify = sub.add_parser("verify", help="unpack an archive elsewhere and try it")
    verify.add_argument("archive", type=Path)
    verify.add_argument("--version", help="default: the workspace version")
    verify.set_defaults(run=cmd_verify)

    sub.add_parser("names", help="print the names packaging uses").set_defaults(run=cmd_names)
    sub.add_parser("check", help="packaging/ agrees with the brand module").set_defaults(run=cmd_check)

    args = parser.parse_args()
    sys.exit(args.run(args))


main()
