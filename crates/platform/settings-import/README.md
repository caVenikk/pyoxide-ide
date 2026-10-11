# settings-import

Reads settings that users keep in other IDEs and returns them as plain data.
Today: PyCharm database data sources and the passwords PyCharm saved for them.
Turning the data into Pyoxide connection profiles and storing the passwords is
done by the caller (`pyoxide-db`), see ADR 0004.

## What is imported from PyCharm

From the project's `.idea/dataSources.xml` (shared) and
`.idea/dataSources.local.xml` (per user), joined by data source UUID:

- name, UUID, driver id and class, JDBC URL;
- user name, read-only flag, auto-commit, authentication method, password
  storage mode;
- driver properties, except those whose name looks like a secret.

JDBC URLs are parsed into neutral fields (host, port, service name or SID,
database, file path, connect descriptor or TNS alias, TLS mode, parameters):

- Oracle: `@//host:port/service`, `@host:port/service`, `@host:port:SID`,
  `@(DESCRIPTION=...)`, `@alias` with `?TNS_ADMIN=`, EZConnect Plus
  (`tcps://`, parameters), LDAP URLs (kept as a connect string);
- PostgreSQL: `//host[:port]/db?params`, `jdbc:postgresql:db`, several hosts
  (the first is used, with a note), `sslmode` and `ssl`;
- SQLite: file paths with `$PROJECT_DIR$` and `$USER_HOME$` expanded,
  `:memory:`, `file:` URLs.

## What is not imported

- Other database engines (MySQL, SQL Server, ...): returned with the reason.
- SSH tunnels, SSL certificate settings outside the URL, schema filters,
  introspection settings, console history.
- The schema model files in `.idea/dataSources/` (they hold no connection
  settings; one that does is merged).
- Anything from PyCharm's application settings other than the password
  settings (`options/security.xml`).

## Passwords

PyCharm keeps passwords in its KeePass database (`c.kdbx` in the IDE config
directory, unlocked with the master key saved in `c.pwd`) or in the OS
credential store. `pycharm::passwords` reads them only when asked:

- KeePass: the saved master key is decoded as the open-source IntelliJ
  Platform does; if that fails (PGP, DPAPI of another user, a changed
  password) the user is asked for the master password.
- OS store (macOS Keychain): only with the user's explicit consent; the OS
  may ask as well. Linux Secret Service entries use an attribute schema the
  keyring backend cannot query; Windows always uses KeePass.
- The service name of a data source password follows the IntelliJ naming
  scheme `IntelliJ Platform <subsystem> — <key>`. The database plugin is not
  open source: the default (`DB`, data source UUID) is a guess. A dry run
  lists the IDE entry titles that mention a data source UUID so the naming
  can be confirmed.

## Privacy

- Only the user's own files are read; nothing is written to PyCharm's files.
- Secret values travel only as `secrets::Secret` (redacted in `Debug`, wiped
  on drop). They never appear in errors, notes, the dry-run report or JSON.
- Passwords found in URLs or driver properties are dropped, with a note.
- Tests use synthetic fixtures with invented values and never touch the real
  keychain or real KeePass files.
