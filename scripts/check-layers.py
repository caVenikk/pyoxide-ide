#!/usr/bin/env python3
# Layer rules of the workspace, checked from `cargo metadata`:
#
# - ADR 0005: platform crates never depend on product crates, and products
#   do not depend on each other.
# - ADR 0022 (modules): a directory in MODULES (crates/database/) is a
#   reusable module between the platform and the products. A module depends
#   on platform crates only; products depend on platform crates and modules;
#   the platform never depends on a module; a module never depends on a
#   product or on another module.
# - ADR 0021, 0022 (database module): the SQL text layer, the metadata model,
#   the session layer and the connection core know no UI toolkit; no crate of
#   the module depends on a driver or on `db-service`; the metadata model does
#   not know SQL syntax; the editor depends on none of the module's crates.
# - ADR 0010 (language services): the model and the client know no UI
#   toolkit; protocol types stay inside the client; platform UI crates
#   depend on the code intelligence traits, never on the client; the
#   editor has diagnostic marks of its own and does not know the model.
# - ADR 0014 (terminal): the `terminal` crate knows no UI toolkit, and only
#   it depends on the emulator (`alacritty_terminal`).
# - ADR 0016 (version control): the model (`vcs`) and the backend (`git`)
#   know no UI toolkit; the model knows no backend; UI crates depend on the
#   model, never on the backend (the workbench composes them).
# - ADR 0015 (debugger): the DAP client and the debugger model know no UI
#   toolkit; UI crates depend on the debugger, never directly on the DAP
#   client.
#
#     python3 scripts/check-layers.py
#     python3 scripts/check-layers.py --self-test   # only the rules' own tests
#
# A crate under crates/platform/ is a platform crate; a crate under a
# directory of MODULES belongs to that module; a crate under any other
# crates/<product>/ directory belongs to that product. Crates directly under
# crates/ are not classified yet and are only listed. Exit code 1 on a violation.
# Every run starts with the self-test of the layer rules on made-up packages.
import json, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"

# ADR 0022: directories of crates/ that are reusable modules, not products.
MODULES = {"database"}

# ADR 0010; stream-framing, dap-client, debugger and py-debug: ADR 0015.
TOOLKIT = "gpui"
NO_TOOLKIT = ["code-intel", "lsp-client", "py-lsp", "settings", "terminal", "stream-framing", "dap-client", "debugger", "py-debug"]
CLIENT = "lsp-client"
PROTOCOL_TYPES = "lsp-types"
MODEL = "code-intel"
NOT_MODEL = ["editor"]

# ADR 0021, 0022: the database module (crates/database/) and the connection
# core it uses. `db-ui` is the module's one crate that has a toolkit.
DB_NO_TOOLKIT = ["sql-syntax", "sql-intel", "db-model", "db-session", "db-core"]
DB_DRIVERS = ["db-postgres", "db-sqlite", "db-oracle-thin", "db-oracle-odpi", "db-service"]
DB_MODEL = "db-model"
SQL_SYNTAX = "sql-syntax"
DB_MODULE_CRATES = ["sql-syntax", "sql-intel", "db-model", "db-session", "db-ui"]
DB_NOT_USERS = ["editor"]

# ADR 0014.
EMULATOR = "alacritty_terminal"
EMULATOR_USER = "terminal"
# ADR 0016.
VCS_MODEL = "vcs"
VCS_BACKEND = "git"
VCS_NO_TOOLKIT = [VCS_MODEL, VCS_BACKEND]
COMPOSER = "workbench"

# ADR 0015.
DAP_CLIENT = "dap-client"
DEBUGGER = "debugger"


def layer(manifest: Path):
    rel = manifest.parent.relative_to(CRATES)
    if len(rel.parts) < 2:
        return None  # crates/<name>: not classified yet
    return rel.parts[0]  # "platform" or a product name


def reach(direct, name):
    """Every workspace crate that `name` reaches through its dependencies."""
    seen, stack = set(), [name]
    while stack:
        for d in direct.get(stack.pop(), ()):
            if d in direct and d not in seen:
                seen.add(d)
                stack.append(d)
    return seen


def uses(direct, name, dependency):
    """`name` depends on `dependency`, directly or through workspace crates."""
    crates = {name} | reach(direct, name)
    return dependency in crates or any(dependency in direct[c] for c in crates)


def database_violations(direct, by_name):
    """The rules of the database module (ADR 0021, 0022). `direct` maps a crate
    to its direct dependencies, `by_name` to its layer."""
    bad = []
    for name in DB_NO_TOOLKIT:
        if name in direct and uses(direct, name, TOOLKIT):
            bad.append(f"{name} uses the UI toolkit ({TOOLKIT}) (ADR 0021)")
    for name in sorted(direct):
        if by_name.get(name) == "database":
            for driver in DB_DRIVERS:
                if uses(direct, name, driver):
                    bad.append(f"{name} (database module) depends on {driver}: the module sees connections through db-core only (ADR 0021)")
    if DB_MODEL in direct and uses(direct, DB_MODEL, SQL_SYNTAX):
        bad.append(f"{DB_MODEL} depends on {SQL_SYNTAX}: the model knows no SQL syntax (ADR 0022)")
    for name in DB_NOT_USERS:
        for dep in DB_MODULE_CRATES:
            if name in direct and uses(direct, name, dep):
                bad.append(f"{name} depends on {dep}: the editor knows nothing of the database module (ADR 0021)")
    return bad


def layer_violations(by_name, packages, modules=MODULES):
    """The ADR 0005 / 0022 rules for `packages` (name and dependencies) given
    the layer of every crate: `platform`, one of `modules`, a product's
    name, or None when not classified."""
    bad = []
    for p in packages:
        own = by_name[p["name"]]
        if own is None:
            continue
        for d in p["dependencies"]:
            dep = by_name.get(d["name"])
            if dep is None or dep == own:
                continue
            name, what = p["name"], d["name"]
            if own == "platform":
                kind = "a module" if dep in modules else "a product"
                bad.append(f"{name} (platform) depends on {what} ({dep}, {kind})")
            elif own in modules:
                if dep == "platform":
                    continue
                kind = "another module" if dep in modules else "a product"
                bad.append(f"{name} (module {own}) depends on {what} ({dep}, {kind}): modules use platform crates only (ADR 0022)")
            elif dep not in modules and dep != "platform":
                bad.append(f"{name} ({own}) depends on {what} (another product: {dep})")
    return bad


def self_test():
    """The layer rules on made-up crates: what must pass and what must fail."""
    def pkg(name, *deps):
        return {"name": name, "dependencies": [{"name": d} for d in deps]}

    layers = {"ui": "platform", "core": "platform", "sql": "database", "sql2": "database", "dbx": "other-mod",
              "app": "python", "other-app": "tools", "loose": None}
    ok = [pkg("ui", "core"), pkg("sql", "core", "ui"), pkg("sql", "sql2"), pkg("app", "ui", "sql", "app"),
          pkg("loose", "app"), pkg("app", "loose")]
    for p in ok:
        got = layer_violations(layers, [p], {"database", "other-mod"})
        assert not got, f"{p} should pass, got {got}"
    fails = [(pkg("ui", "sql"), "a module"), (pkg("core", "app"), "a product"),
             (pkg("sql", "app"), "a product"), (pkg("sql", "dbx"), "another module"),
             (pkg("app", "other-app"), "another product")]
    for p, word in fails:
        got = layer_violations(layers, [p], {"database", "other-mod"})
        assert len(got) == 1 and word in got[0], f"{p} should fail with '{word}', got {got}"

    # The database module's own rules, on made-up dependency graphs.
    names = ["sql-syntax", "sql-intel", "db-model", "db-session", "db-core", "db-ui", "db-postgres", "db-service", "editor", "gpui", "wb"]
    module_layers = {n: "database" for n in ["sql-syntax", "sql-intel", "db-model", "db-session", "db-ui"]}
    module_layers.update({n: "platform" for n in ["db-core", "db-postgres", "db-service", "editor", "gpui", "wb"]})

    def graph(**edges):
        direct = {n: set() for n in names}
        for k, v in edges.items():
            direct[k.replace("_", "-")] = set(v)
        return direct

    good = graph(sql_intel=["sql-syntax", "db-model"], db_session=["db-core", "sql-syntax"], db_ui=["sql-intel", "gpui", "editor"],
                 db_service=["db-postgres", "db-core"], wb=["editor", "gpui", "db-service"])
    got = database_violations(good, module_layers)
    assert not got, f"the module rules should pass, got {got}"
    bad_graphs = [
        (graph(sql_syntax=["gpui"]), "sql-syntax uses the UI toolkit"),
        (graph(sql_intel=["db-model"], db_model=["gpui"]), "db-model uses the UI toolkit"),
        (graph(db_session=["sql-intel"], sql_intel=["gpui"]), "sql-intel uses the UI toolkit"),
        (graph(db_core=["gpui"]), "db-core uses the UI toolkit"),
        (graph(db_session=["db-postgres"]), "db-session (database module) depends on db-postgres"),
        (graph(db_ui=["db-session"], db_session=["db-service"]), "db-ui (database module) depends on db-service"),
        (graph(db_model=["sql-syntax"]), "db-model depends on sql-syntax"),
        (graph(db_model=["db-session"], db_session=["sql-syntax"]), "db-model depends on sql-syntax"),
        (graph(editor=["sql-syntax"]), "editor depends on sql-syntax"),
        (graph(editor=["wb"], wb=["db-ui"]), "editor depends on db-ui"),
    ]
    for direct, what in bad_graphs:
        got = database_violations(direct, module_layers)
        assert any(what in g for g in got), f"expected '{what}', got {got}"


def main():
    try:
        self_test()
    except AssertionError as e:
        print(f"layer rules self-test failed: {e}")
        sys.exit(1)
    if "--self-test" in sys.argv:
        print("layer rules self-test ok")
        return
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    by_name = {}
    for p in meta["packages"]:
        by_name[p["name"]] = layer(Path(p["manifest_path"]))
    unclassified = [p["name"] for p in meta["packages"] if by_name[p["name"]] is None]
    bad = layer_violations(by_name, meta["packages"])

    # What each workspace crate uses to build (dev-dependencies left out):
    # its direct dependencies, and every crate it reaches in the workspace.
    direct = {
        p["name"]: {d["name"] for d in p["dependencies"] if d.get("kind") != "dev"}
        for p in meta["packages"]
    }

    def uses_(name, dependency):
        return uses(direct, name, dependency)

    for name in NO_TOOLKIT:
        if name in direct and uses_(name, TOOLKIT):
            bad.append(f"{name} uses the UI toolkit ({TOOLKIT}) (ADR 0010)")
    for name in sorted(direct):
        if name != CLIENT and PROTOCOL_TYPES in direct[name]:
            bad.append(f"{name} depends on {PROTOCOL_TYPES}: protocol types stay in {CLIENT} (ADR 0010)")
        if by_name.get(name) == "platform" and uses_(name, TOOLKIT) and uses_(name, CLIENT):
            bad.append(f"{name} is a UI crate and depends on {CLIENT}: UI crates use {MODEL} (ADR 0010)")
    for name in sorted(direct):
        if name != EMULATOR_USER and EMULATOR in direct[name]:
            bad.append(f"{name} depends on {EMULATOR}: only {EMULATOR_USER} sees the emulator (ADR 0014)")
    # Direct dependencies only: a UI crate reaches the client through the
    # debugger, which is the point.
    for name in sorted(direct):
        if name != DEBUGGER and DAP_CLIENT in direct[name] and uses_(name, TOOLKIT):
            bad.append(f"{name} uses the UI toolkit and depends on {DAP_CLIENT}: UI crates use {DEBUGGER} (ADR 0015)")
    for name in NOT_MODEL:
        if name in direct and uses_(name, MODEL):
            bad.append(f"{name} depends on {MODEL}: it has diagnostic marks of its own (ADR 0010)")

    for name in VCS_NO_TOOLKIT:
        if name in direct and uses_(name, TOOLKIT):
            bad.append(f"{name} uses the UI toolkit ({TOOLKIT}) (ADR 0016)")
    if VCS_MODEL in direct and uses_(VCS_MODEL, VCS_BACKEND):
        bad.append(f"{VCS_MODEL} depends on {VCS_BACKEND}: the model knows no backend (ADR 0016)")
    # The workbench composes the UI crates with the backend; a crate built
    # on the workbench (the test fixture of a module) is composed above it.
    for name in sorted(direct):
        if (by_name.get(name) == "platform" and name != COMPOSER
                and COMPOSER not in direct[name]
                and TOOLKIT in direct[name] and uses_(name, VCS_BACKEND)):
            bad.append(f"{name} is a UI crate and depends on {VCS_BACKEND}: UI crates use {VCS_MODEL} (ADR 0016)")

    bad += database_violations(direct, by_name)

    if unclassified:
        print("not classified yet:", ", ".join(sorted(unclassified)))
    if bad:
        print("layer violations (ADR 0005, 0010, 0014, 0015, 0016, 0021, 0022):")
        for b in bad:
            print("  " + b)
        sys.exit(1)
    print("layers ok")


main()
