# Rebranding

The product name is spelled out in one place: the `brand` crate
(`crates/python/brand/src/lib.rs`, product layer). Everything else takes it
from there:

- Product crates (`crates/python/`, `db-cli`) use the `brand` constants and
  `brand::env_name!("X")` (compile time, for `concat!` and attributes).
- Platform crates and modules know no name (ADR 0005). They read the
  `app_info::AppInfo` the product installs at startup (`brand::install()` in
  `main`, `Product::app_info` in `workbench::run`); until then it is a neutral
  one (`App`, slug `app`, prefix `APP`, folder `.app`), which is what the tests
  of platform crates run with.
- `python3 scripts/check-brand.py` (part of `scripts/check.sh` and CI) fails on
  the name in the code of `crates/` outside `brand`, apart from comments, test
  code, `Cargo.toml` and a short allow-list in the script, each entry with its
  reason.

## What `brand` holds

| Constant | Today | Where it shows up |
|---|---|---|
| `PRODUCT_NAME` | `Pyoxide` | window title, menus, messages, KeePass group, `TERMINAL_EMULATOR` |
| `PRODUCT_FULL_NAME` | `Pyoxide IDE` | LSP and DAP client name, Oracle session module, `--help` |
| `SLUG` | `pyoxide` | configuration and cache folders, database `application_name`, thread names, temporary files, base theme ids (`pyoxide-dark`, `pyoxide-light`), keychain service prefix, save-file marker |
| `ENV_PREFIX` | `PYOXIDE` | every environment variable (`PYOXIDE_CONFIG_DIR`, `PYOXIDE_THEME`, `PYOXIDE_DB_PASSWORD`, ...) and the `PYOXIDE <name> <json>` measurement lines |
| `PROJECT_DIR` | `.oxide` | the product's folder in a project |
| `BUNDLE_ID` | `dev.pyoxide.ide` | application bundle identifier (no bundle is built yet) |
| `CLI_NAME`, `DB_CLI_NAME`, `IDE_NAME` | `pyoxide`, `pyoxide-db`, `pyoxide-ide` | command-line texts, the keychain service of database passwords (`pyoxide-db`), the programs trusted by keychain items |

## Changing a value is a migration

The values are what existing installations have on disk. Before changing one
decide what happens to the old data, or existing users lose it:

- `SLUG`: the settings folder (`<config dir>/pyoxide/`), the cache folder, the
  keychain items of `<slug>-db` and `<slug>-keepass`, the stored theme id.
- `PROJECT_DIR`: `.oxide/` in every project (shared settings, run
  configurations, personal state).
- `ENV_PREFIX`: environment variables set in users' shells and scripts.
- `DB_CLI_NAME`: the keychain service of the data source passwords.

Either keep the old value where it is only a storage name, or read the old
location once and write the new one.

## Manual steps

These are not generated from `brand`; rename them by hand and keep them equal
to the `brand` values.

- [ ] `crates/python/brand/src/lib.rs`: the values above.
- [ ] Package names (`pyoxide-ide`, `pyoxide-cli`) and their directories, the
      `[[bin]]` names (`pyoxide-ide`, `pyoxide`, `pyoxide-db`) in the
      `Cargo.toml` files, the crate paths (`pyoxide_ide::...`), `description`
      fields. `brand::IDE_NAME`, `CLI_NAME` and `DB_CLI_NAME` must equal the
      binary names (keychain items trust those binaries).
- [ ] `repository` in the workspace `Cargo.toml`, the GitHub repository, issue
      templates, links in `README.md` and `CONTRIBUTING.md`.
- [ ] Scripts: `scripts/sign-local.sh` (identifiers follow the binary names),
      `scripts/seed-target.sh`, `scripts/fetch-debugpy.py` (computes the cache
      folder `<slug>`), `scripts/perf/pyoxide_bench.py` (reads the environment
      variables and the `PYOXIDE <name> <json>` lines), `scripts/progress.py`.
- [ ] `.github/workflows/`: artifact and binary names.
- [ ] Packaging, once there is any: bundle identifier, `Info.plist`, installer
      ids, code-signing identity (`docs/code-signing.md`).
- [ ] Development databases: `docker-compose.yml` and the tests use a database
      and user called `pyoxide`; these are fixtures, not the product name.
- [ ] Documentation: `README.md`, `docs/` (including the file name
      `docs/pyoxide-db.md`), ADRs, `skills/`, `replica/`, `notices/`.
- [ ] Comments and doc comments in the code: `grep -rni pyoxide crates`
      (`check-brand.py` ignores comments).
- [ ] Tests: they spell the names out where they test them, and platform
      crates' tests run with the neutral names (`app`, `APP`, `.app`).
      `grep -rni pyoxide crates/*/*/tests` and the `#[cfg(test)]` modules.
- [ ] Icons and logo: `crates/platform/ui/assets`, the SVG headers that name
      the product.
- [ ] Internal protocol names carry no product name on purpose (`__IDE_*`
      variables and `shell-integration.*` of the terminal scripts, the savepoint
      `px_stmt`, the `PX$ERROR` and `PX$NATIVE` columns of the Oracle driver).
