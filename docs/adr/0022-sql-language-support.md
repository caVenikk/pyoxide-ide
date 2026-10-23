# 0022. SQL language support

- Status: proposed
- Date: 2026-10-23

## Context
ADR 0021 plans the database interface. SQL is also a language of the
editor: the reference IDE highlights it, completes names from the metadata of
a data source, reports unresolved names and syntax errors, offers quick fixes,
shows documentation of tables and routines, navigates to declarations,
formats by a code style, and recognises SQL inside string literals of other
languages (`replica/recon.md`, area `db-gui`, section "SQL language"). In the
reference these come from the database tools plugin, which is the same in the
Python IDE and in the database IDE; the Python IDE does not own them.

Constraints:
- The platform knows no language and never depends on products (ADR 0005).
  A SQL language engine is not a platform concern, and a database client is
  not needed by every product, but more than one product will want it: the
  Python IDE now, a database IDE later.
- Language features reach the editor through the `CodeIntel` trait and
  `code-intel-ui` (ADR 0010). Today a product gives the workbench exactly one
  provider, and a document is routed by nothing (the one provider serves all
  language ids). ADR 0005 said to add the language extension point when a
  second language appears; SQL is the second language.
- Completion and resolution need the metadata model (ADR 0021) at once, also
  before any connection, so they cannot sit behind a process boundary that
  has no access to the model.
- Text being typed is almost never valid: `select c. from customers c` is the
  usual state when completion is wanted. The parser must give a useful tree
  for it.
- Budgets of `docs/performance.md` apply: the UI thread never waits; the
  completion popup shows within 100 ms of the answer.
- Clean-room (`docs/clean-room.md`): behaviour comes from public
  documentation and observation; the database plugin is closed source and
  its binaries are never opened. The Apache-2.0 platform sources of the
  reference (language injection infrastructure, completion and navigation
  UX) may be read.

## Options considered

### Where the SQL services run
1. A language server over LSP (an existing SQL server, or our own binary).
   Existing ones cover PostgreSQL at most, know nothing of our metadata
   cache, consoles and bindings, and cost a process and a round trip per
   keystroke. Our own binary would need the model across a protocol.
2. Inside `db-ui` as ad hoc functions (completion in the console editor).
   Fast to start, but then SQL files, grid filter fields, injected strings
   and a second product each reimplement it, and the editor-side features
   (diagnostics, quick fixes, hover) bypass `CodeIntel`.
3. In-process, as another implementation of `CodeIntel`, over the model.
   Chosen.

### Parser
Evidence was gathered by cloning the candidates and running them on
sample statements (mid-typing states, dialect constructs).

| candidate | licence | dialects | error tolerance | verdict |
|---|---|---|---|---|
| `tree-sitter-sql` (`tree-sitter-sequel` on crates.io, 0.3.11) | MIT | one generic grammar, MySQL / PostgreSQL / BigQuery leaning; no Oracle, no PL/SQL, no SQLite-specific statements | tree-sitter recovers, but the shape around the error is a guess: `select c. from customers c` parses `from` as a field name and puts `c` in an `ERROR`; `insert .. on conflict` (PostgreSQL, SQLite) is an `ERROR` | usable for colouring only; wrong for completion and resolution |
| supabase postgres-language-server grammar (a fork of the above) | MIT | PostgreSQL only | designed for statements being typed (typed identifier nodes), 3.9k lines of grammar | a design reference for completion contexts; PostgreSQL only |
| `sqlparser` 0.63 | Apache-2.0 | generic, PostgreSQL, SQLite, Oracle, others | none: the first error aborts, no tree for `select c. from ..` or an unclosed `(`; no PL/SQL blocks; `begin update ..; end;` is an error | not a parser for an editor; useful as a test oracle for valid statements |
| `libpg_query` (PostgreSQL's own parser, C) | BSD-3-Clause | PostgreSQL only, exact | first error only; C code of 11 MB of sources | a test oracle for the PostgreSQL grammar, not shipped |
| Own recursive-descent parser per dialect producing a lossless tree | ours | the three engines and generic, by tables and flags | by design: statement-level units, clause-level recovery, expected-token sets, opaque nodes for what is not understood | chosen |

Own parser costs more than adopting a grammar, but the three things the
features depend on are exactly what the grammars lack: a tree for broken
statements that keeps `FROM` where it is, the set of tokens that may follow
the caret (completion), and PL/SQL and PostgreSQL's extensions. The cost is
bounded by staging (below) and by never reporting what is not understood:
a statement kind the parser does not know is one opaque node with its tokens,
no diagnostics, highlighting by the lexer.

## Decision

### Module layer
A third kind of layer joins the platform and the products (this amends ADR
0005). A **module** is a reusable set of crates that a product may include:
it is the equivalent of a plugin of the reference that several of its
products share, composed at compile time like everything else.

- `crates/database/` is the first module: `sql-syntax`, `sql-intel`,
  `db-model`, `db-session`, `db-ui` (ADR 0021). The connection engine
  (`db-core`, the drivers, `db-service`, `db-cli`, `db-testkit`) stays in
  `crates/platform/`: it is a connection layer and knows no language.
- Rules, enforced by `scripts/check-layers.py` from `cargo metadata`: a
  module depends on platform crates only; products depend on platform
  crates and on modules; the platform never depends on a module; a module
  never depends on a product or on another module. Products still do not
  depend on each other. The script has a self-test of these rules.
- A module reaches the application through `workbench::Module`, a plain
  struct of the same shape as the contribution fields of `Product`
  (file types and associations, icons, settings pages, status widgets, the
  `init` function that registers commands, tool windows and globals, keymap
  files, code intelligence for languages, injection sources). `Product`
  gets `with_module(Module)`, which merges them in order: lists are
  appended after the product's own, the product's file types still win over
  the module's, `init` functions run in order. The module needs two
  inputs only a product can supply, and takes them as arguments of its
  constructor (`db_ui::module(config)`): a `db_core::ConnectionSource`
  (built from `db-service`) and the `DbRuntime`.
- Keymap files of a module live in its crate
  (`crates/database/db-ui/keymaps/{macos,default}/db.toml`) and are joined
  after the workbench's; their keystroke checks are integration tests of
  `db-ui` that use the workbench's keymap harness, which the workbench
  exports behind a `test-support` feature. The platform's own tests do not
  depend on a module.
- A product without the module has no Database tool window, no SQL
  language and does not link the crates.

### Crates and dependencies
| crate | layer | toolkit | role |
|---|---|---|---|
| `sql-syntax` | database | no | the SQL text layer: lexer, dialect tables, statement splitting, classification, parameter scan, the lossless error-tolerant syntax tree and its statement-level incremental reparse, the formatter, the explain wrapper. Depends on nothing but `std` (and small utility crates). |
| `sql-intel` | database | no | the SQL language services over `db-model`: scopes and reference resolution, completion, inspections and quick fixes, quick documentation, parameter info, navigation, usages and rename inside a file, structure, the injection engine (virtual documents), code style, `SqlCodeIntel: CodeIntel`. Depends on `sql-syntax`, `db-model`, `code-intel`, `name-match`, `text`. |
| `db-model` | database | no | as ADR 0021; knows no SQL syntax. |
| `db-ui` | database | yes | gets the SQL editor glue: the SQL file type, the editor highlighter (lexer spans at once, tree roles from a background pass, mapping of injected fragments), the dialect and scope widget, the Settings pages (SQL Dialects, SQL Resolution Scopes, Code Style > SQL, Inspections), the bindings of files and consoles to data sources. |

Dependencies: `sql-intel` -> `sql-syntax`, `db-model`, `code-intel`;
`db-session` -> `sql-syntax`; `db-ui` -> `sql-intel`, `sql-syntax`,
`db-session`, `db-model`. `sql-syntax` and `db-model` do not depend on each
other: identifiers are compared by `sql-intel`, which applies the dialect's
case folding and quoting rules (`Dialect::fold`, `Dialect::quote`) to the
model's exact names.

### Routing documents to code intelligence providers
The first language extension point (ADR 0005), in the platform:

- `CodeIntelHook` (the product's provider, ADR 0010) gets
  `languages() -> &[&'static str]`: the language ids it serves; the empty
  list serves every id, as today. `Product.code_intel` and
  `Module.code_intel` become lists.
- `code-intel` gets `RoutedCodeIntel`, a `CodeIntel` over several
  providers. A document goes to the providers whose languages contain its
  language id (`FileType.language_id`; the SQL file type gets `"sql"`). The
  document lifecycle calls (`did_open`, `did_change`, ...) reach those
  providers only; project-wide calls (`files_changed`, `restart`,
  `shutdown`) reach all; `status()` concatenates, `events()` merges into
  one hub. Requests for a document go to its providers: list-valued
  answers (completion, diagnostics, code actions, references) are
  concatenated in provider order, single answers (hover, definition,
  signature help, format, rename) come from the first provider that does
  not answer `Unsupported` or empty. Two providers for one language
  (a future SQL language server next to ours) work through the same rules.
- `code-intel-ui` asks the hook that serves the document's language for
  the UI-level answers it asks of the hook today (`parentheses`, `lexer`,
  `doc_location`, icons).
- Additions to the trait, with defaults that answer `Unsupported`:
  `document_symbols(doc)` (structure view, also wanted for Python) and
  `definition_targets(doc, at)`, whose default maps `definition` to
  file targets. A target is a file location or an *object target*
  `{ scheme, id, title }`; the workbench has a registry of object-target
  handlers by scheme, and the database module registers `db` (selects the
  node in the Database tool window, or opens the object's DDL as a
  read-only tab when the tool window is hidden or the object is a view or
  routine with source).
- Injected languages (below) are served by the same routing: the routing
  layer owns the virtual documents.

### Dialects
`Dialect` is `Generic`, `Postgres`, `Oracle`, `Sqlite`. A dialect is data
and a few flags in `sql-syntax`, not a trait with behaviour spread around:

- lexical rules: identifier characters and quoting (`"x"`, backticks off),
  string forms (`'..'` with doubled quote, PostgreSQL `E'..'`, `$tag$..$tag$`,
  Oracle `q'[..]'`, `N'..'`), numbers, comments (nested block comments on
  PostgreSQL), parameter forms (`?`, `$1`, `:name`, `@name`, `%s` and
  `%(name)s` for injected Python, `${name}`), operators (`::`, `->>`, `@>`,
  `||`, `:=`, `=>`), statement terminators and the Oracle `/` line;
- reserved and non-reserved keywords from the engines' public manuals;
- grammar flags (`returning`, `on conflict`, `merge`, `connect by`,
  `lateral`, `ilike`, `limit` versus `fetch first`, `pivot`, upsert forms,
  window and CTE options), so one parser serves all dialects and a flag
  turns a construct on;
- the built-in function and type tables (name, signature, short
  description in our own words from the public manuals), used by
  completion, parameter info and documentation;
- case folding and quoting of identifiers (PostgreSQL folds to lower case,
  Oracle to upper case, SQLite compares case-insensitively).

`Generic` accepts the union of the common syntax and treats dialect
constructs it does not know as opaque rather than as errors. Oracle covers
SQL and PL/SQL; PostgreSQL covers the SQL grammar, `DO` blocks and
PL/pgSQL bodies; SQLite covers its statements, `PRAGMA` and trigger
bodies. More dialects are added as tables, not as parsers.

### How a document gets its dialect
First match wins:
1. A console: the dialect of its data source (`postgres` -> `Postgres`,
   `oracle-thin` and `oracle-odpi` -> `Oracle`, `sqlite` -> `Sqlite`).
2. A file bound to a data source (personal state, ADR 0009): the same.
3. A mapping of the file or a folder above it to a dialect, the closest
   path wins. Shared mappings are in `.oxide/project.toml`, personal ones in
   the personal project state.
4. The project default dialect (`[sql] dialect`).
5. `Generic`.

```toml
# .oxide/project.toml
[sql]
dialect = "postgres"             # project default; "generic" when absent

[[sql.mappings]]
path = "warehouse/oracle/"       # a file or a folder, relative to the project
dialect = "oracle"
```

The status bar shows the dialect of the current SQL file and changes its
mapping from a popup (personal by default, shared on request); the
Settings page *SQL Dialects* lists and edits the mappings. Switching the
dialect re-parses the document.

### Resolution scope
A **scope** says what names in a document mean: a data source and an
optional schema or search path. Sources, first match wins:
1. A console: its data source and the session's current schema or
   search path (the schema chip; `set search_path` typed by hand updates it).
2. A file bound to a data source and schema.
3. A mapping of the file or a folder above it to a data source and schema
   (`[[sql.mappings]]` with `data-source` and `schema`, closest path wins;
   the data source is an ADR 0021 `DataSourceId`).
4. The project default scope (`[sql]` `data-source`, `schema`).
5. No scope.

Rules:
- Resolution reads the immutable `Arc<Model>` of the data source, from the
  disk cache before any connection exists. It never touches the network.
- Unqualified names resolve through the search path (PostgreSQL: the
  session's, else `"$user", public`; Oracle: the current schema, then
  public synonyms; SQLite: `main`, `temp`, attached). Qualified names
  resolve exactly.
- The result of resolving a name is four-valued: `Resolved`, `Ambiguous`,
  `Unresolved`, **`Unknown`**. A name is `Unknown`, never `Unresolved`,
  when the answer cannot be known: no scope; the schema is outside the data
  source's introspection scope; the schema is loaded to names only so its
  columns are not known; a fragment contains a hole (below); the dialect is
  `Generic`. Only `Unresolved` is reported. A document without a scope gets
  syntax errors and keyword completion but no resolution inspections.
- A script is its own scope as well: objects created earlier in the same
  script or console (`create table`, `create view`, `create function`,
  CTEs, temporary tables) resolve to their definitions and complete
  without a model.
- Completion may offer objects of schemas outside the search path with the
  schema inserted (setting *Show objects from all schemas*, on for
  completion, off for the unresolved inspection).
- Which scope a document has is shown beside the dialect in the SQL
  widget; the bound data source's connection state does not matter.

### Syntax tree
`sql-syntax` produces a **lossless tree**: every byte of the statement
belongs to a token; whitespace and comments are trivia attached to tokens;
printing the tree gives the input back (property test). Parse units are
statements, split by the lexer (not by searching for `;`): strings,
quoted identifiers, comments, `$tag$` quotes, `q'[..]'` and PL/SQL blocks
do not split. The tree of a script is a list of statement trees with byte
ranges; an edit re-lexes and re-parses the statements it touches and shifts
the others, so the cost of a keystroke is the size of one statement.

- Nodes have a kind from an enum (select, from item, join, expression
  kinds, name parts, create table, column definition, call, ...), a byte
  range and children; names are typed by position (`TableRef`, `ColumnRef`,
  `SchemaRef`, `FunctionRef`, `AliasDef`, `CteDef`, `TypeRef`), because
  the parser knows what a name is in `FROM` or after `.`.
- Error recovery: an `Error` node holds the unexpected tokens and the set
  of tokens that would have been valid (the *expected set*); recovery
  resumes at the next clause keyword, closing parenthesis or statement end;
  missing tokens are recorded as `Missing` nodes with no text. `select
  c. from customers c` yields a select with a select item whose qualifier
  `c` is followed by a missing name, and the `FROM` clause intact: that is
  the shape completion needs.
- `Opaque`: a statement or clause the parser does not know is one node
  with its tokens and no errors. The grammar is staged (below), so
  unsupported syntax degrades to the lexer's colours, never to false
  errors.
- `Hole`: a token that stands for text outside SQL (an injected host
  expression). The parser accepts a hole where an identifier, expression
  or list item may stand, and as a whole clause in clause position.
- Queries on the tree: `node_at(offset)`, `expected_at(offset)`,
  `enclosing(kind)`, and scope-visiting helpers used by resolution.
- Offsets are bytes in the document; `sql-intel` converts to
  `text::Point` at the `CodeIntel` boundary.

Staging of the grammar (each stage is usable alone; the first is the
lexer-only fallback): (1) lexer, statement splitter, classification;
(2) queries: select, set operations, CTEs, joins, subqueries, window
functions, expressions, the three dialects' query extensions; (3) DML:
insert (with `returning`, `on conflict`, Oracle multi-table), update,
delete, merge; (4) DDL: create / alter / drop for tables, views, indexes,
sequences, schemas, types; (5) procedural: PostgreSQL `$$` bodies and `DO`,
Oracle PL/SQL blocks (declarations, control flow, cursors, exceptions,
procedures, functions, packages, triggers), SQLite trigger bodies. Test
oracles: `libpg_query` for valid PostgreSQL statements and `sqlparser` for
the common subset, as dev-dependencies or as corpus generators (licences
recorded in the issue).

### Highlighting
Three layers, as in the `syntax` crate's design (the lexer never waits for
the tree):
1. **Lexical**, synchronous per line with a small carried state (inside a
   block comment, a string, a `$tag$` body, a `q'[` string): keywords,
   strings, numbers, comments, operators, parameters, quoted identifiers.
   Also implements `syntax::Lexer` so documentation code blocks reuse it.
   Budget: a visible window is coloured before the next paint.
2. **Syntactic**, from the tree after a background pass (the statement
   being edited first): table, column, alias, schema and CTE names,
   function calls versus keywords, types, parameters. New `themes` text
   roles: `SqlTable`, `SqlColumn`, `SqlAlias`, `SqlSchema` with fallbacks
   (table -> class, column -> attribute, alias -> variable, schema ->
   module); the reference's colour-scheme keys for them are mapped when the
   importer meets them (`docs/theme-mapping.md`).
3. **Semantic**: unresolved and ambiguous names, unused CTEs, as
   inspections (diagnostic marks), not as colours.

The highlighter is an `editor::Highlighter` in `db-ui`. There is no
tree-sitter grammar for SQL.

### Syntax errors
Reported from `Error` and `Missing` nodes after the editor's 300 ms
highlighting delay (ADR 0010), message `Expected: <up to five tokens>` or
`Unexpected '<token>'`, severity error, range on the offending token (for a
missing token, the end of the previous one). Nothing is reported inside
`Opaque` nodes. An unterminated string or comment is reported at its
start. In injected fragments with holes only errors that cannot be caused
by a hole are reported (below).

### Inspections and quick fixes
Inspections run on the statements changed since the last run, in a
background task, and are published as `IntelEvent::Diagnostics` of the
document with a code (an id). Severities are settings; the defaults below
are our choice and need a reference screenshot to compare ("verify").

First set:
| id | what | default |
|---|---|---|
| `SqlSyntax` | syntax error (above) | error |
| `SqlUnresolved` | table, view, schema, column or routine that does not resolve (`Unresolved` only) | error (verify) |
| `SqlAmbiguous` | an unqualified column found in more than one table of the `FROM` | error |
| `SqlNoWhere` | `DELETE` or `UPDATE` without `WHERE` (shares `sql-syntax` classification) | warning |
| `SqlInsertArity` | `INSERT` with a different number of columns and values | error |
| `SqlNullComparison` | `= NULL`, `<> NULL`, `!= NULL` | warning |
| `SqlUnusedCte` | a common table expression never referenced | weak warning, greyed |
| `SqlUnsupportedSyntax` | a construct another dialect has, in a document whose dialect is known | weak warning (later) |

Later: columns missing from `GROUP BY`, ambiguous `JOIN` without condition,
`SELECT *` in `INSERT .. SELECT`, deprecated functions, type mismatches.

Quick fixes (code actions of kind `quickfix`, offered at the mark):
- `SqlUnresolved`: *Change to 'x'* for the nearest names (one fix per
  candidate, by edit distance and `name-match`); *Qualify with schema*
  when the object exists in another schema; *Use alias* inside a statement.
- `SqlAmbiguous`: *Qualify with 'a'* per candidate table or alias.
- `SqlNoWhere`: *Add WHERE clause* (inserts `where ` and leaves the caret
  after it).
- `SqlNullComparison`: *Replace with IS NULL / IS NOT NULL*.
- `SqlUnusedCte`: *Remove CTE*.
- `SqlSyntax`: *Add missing ')'* when the expected set is exactly a
  closing parenthesis.
- Every inspection: *Suppress for statement* adds `-- noinspection <id>`
  before the statement; *Disable inspection* writes the setting.

### Quick documentation and parameter info
Quick documentation (`hover`, Markdown, same popup as other languages):
a table or view shows its kind icon, qualified name, comment, columns
(name, type, key marks, nullability) and the DDL link; a column shows
type, nullability, default, key and foreign-key target, comment; a routine
its signature, return type, language and comment; an alias or CTE says what
it stands for and shows its columns; a keyword or a built-in function shows
the signature and one line of our own text. Texts come from the model
(comments) and the dialect tables.

Parameter info (`signature_help`): calls of routines from the model and
built-in functions of the dialect: all overloads, the active parameter by
commas at nesting depth, named arguments (`=>`, `:=`). Later: the target
column of the value under the caret in `INSERT .. VALUES`.

### Navigation, usages, rename, structure
- Go to declaration (`definition_targets`): a table alias to its place in
  `FROM`; a CTE name to its definition; a select-list alias used in
  `ORDER BY` / `GROUP BY` to the item; a column to its table's column
  (object target, `db` scheme) and a table, view, routine or schema to its
  object (object target); an object created in the same script to its
  `create` statement; a PL/SQL variable or parameter to its declaration.
- Find usages and highlight usages (`references`, `document_highlights`)
  for aliases, CTEs, PL/SQL variables and parameters, and for columns and
  tables inside the current file (and injected fragments of it). Usages of
  database objects across project files need an index and come later.
- Rename (`rename`) for aliases, CTEs, PL/SQL variables and parameters, and
  script-local objects, within the file. Renaming database objects is a
  database refactoring, out of scope.
- Structure (`document_symbols`): statements with their kind and name
  (`create table orders`, CTE names, procedures, functions and packages with
  their members), and the SQL statements of an injected fragment under the
  host's symbol.

### Formatting and code style
The formatter works from the tree, per dialect, for a statement or all
statements of a range (whole statements only in the first version), as the
`format` of `CodeIntel` and the Reformat Code command.

- Invariants, tested by property tests: the token sequence is unchanged
  apart from whitespace and the case of keywords; strings, quoted
  identifiers, comments, `$$` bodies and Oracle `q'[..]'` strings are not
  touched; the formatter is idempotent; a statement with syntax errors or
  an `Opaque` node is left as it is.
- Code style settings, shared (`.oxide/project.toml`) with per-dialect
  overrides, edited on *Editor > Code Style > SQL*:

```toml
[sql.code-style]
keyword-case = "upper"            # upper | lower | keep
identifier-case = "keep"          # keep only for quoted names
indent = 4                        # spaces; follows the editor when absent
right-margin = 100
comma = "trailing"                # trailing | leading
select-items = "one-per-line"     # one-per-line | wrap
join-layout = "indent"            # indent | align
and-or = "line-start"             # line-start | line-end
blank-lines-between-statements = 1
keep-existing-line-breaks = false
parentheses-padding = false

[sql.code-style.oracle]
keyword-case = "upper"
```

  The exact option list is settled by observing the reference's SQL code
  style page; every option has a test that shows its effect.
- Comments stay attached to the tokens they were attached to. The
  formatter is also used for *Reformat selection* in the console.

### Completion
`completion(doc, at, trigger)` runs on the tree around the caret and the
model; it never waits for a connection.

Contexts, from the expected set and the enclosing nodes:
- after `select` / `where` / `group by` / `order by` / `having` / `on`:
  columns of every table and alias in `FROM` (found even when `FROM` comes
  after the caret), aliases of the select list, functions, keywords that
  may start an expression, `*`;
- after `alias.` or `table.`: columns of that source; after `schema.`:
  its tables, views, routines; `*` is offered after a qualifier;
- after `from` / `join` / `update` / `insert into` / `into` / `table`:
  tables, views, synonyms, CTEs, schemas, table functions;
- after `on` of a join: *join conditions* from foreign keys between the new
  table and the tables before it (`o.customer_id = c.id`, one item per
  key, inserted whole), then columns;
- `insert into t (|)`: columns of `t` not yet listed; `update t set |`:
  columns of `t`; `create index .. on t (|)`: columns; DDL: data types and
  constraint keywords; routine arguments: parameter names (`name =>`);
- keywords by context from the expected set (not a fixed list): only what
  the grammar accepts at the caret, in the case of the code style;
- inside a hole-bearing injected fragment, names adjacent to a hole yield
  keywords only.

Behaviour:
- Items carry kind, detail (type, schema) and documentation resolved
  lazily (`resolve_completion`); names that need quotes are inserted quoted;
  an inserted table gets an alias on request (setting, off by default,
  generated from the table name's initials, unique in the statement).
- Matching and ranking use `name-match` (camel humps, word starts): exact
  prefix first, then objects of the scope's schema, then recently used in
  this session, then other schemas (inserted with schema qualification).
  Objects of the same name in several schemas are told apart by detail.
- Triggers: `.` and, as in other languages, typing letters (the editor's
  setting). Second invocation widens to all schemas.
- The same completion serves the grid's WHERE / ORDER BY fields: a
  document fragment `select * from <table> where |` built from the table
  of the grid, with the field's text as the tail.

### SQL in other languages' strings
The **engine** is in the module, the **detection** in the product.

Platform (`code-intel`, `workbench`): the injection mechanism, which
knows no language.
- A product (or module) supplies an `InjectionSource` for a host language:
  `fragments(doc, text) -> Vec<Fragment>`, run off the UI thread when the
  host text changes. A `Fragment` has a language id (`"sql"`), the
  host pieces and optional hints (dialect, `strict`). A host piece is
  `Literal { host_range, escapes }` (the string content bytes in the host
  document, with the escape sequences of the host language) or
  `Hole { host_range }` (a substitution: `{expr}` of an f-string, `%s`,
  `%(name)s`, `{}` and `{name}` of `format`, a concatenated expression).
- The routing layer builds a **virtual document** per fragment
  (`<host path>#sql:<n>`): its text is the decoded literal pieces with
  one placeholder character (a Unicode private-use character, lexed as a
  `Hole` token) per hole, plus an **offset map** from virtual byte ranges
  to host ranges. Escapes decode to their characters and map as one unit;
  a position inside an escape sequence has no virtual position; host edits
  that touch an escape re-create the fragment.
- Requests at a host position inside a fragment are forwarded to the
  virtual document (position mapped in), and results are mapped back:
  text edits (an edit that would land inside an escape or on a hole is
  dropped; inserted text is escaped for the host literal), locations,
  diagnostics ranges, completion replacement ranges, highlights. Results of the
  host language's own providers and of the virtual document are merged.
  Diagnostics of a fragment are published as part of the host document's.
- Highlighting: the workbench wraps the host file type's highlighter with
  an overlay that paints the SQL spans of fragments (mapped back) over the
  host's string colour; holes stay in the host's colour.
- Reformat inside a fragment re-escapes and keeps the indentation of the
  host string; not in the first version.

Module (`sql-intel`): the virtual documents with language `"sql"` are
ordinary SQL documents; a hole is a token the parser accepts in name,
expression and clause positions and resolution treats as `Unknown`. A
fragment with holes is **non-strict** (as the reference's injection of
a formatted string is "frankenstein"): syntax errors and unresolved
references that a hole could explain are not reported; a fragment without
holes is **strict** and gets every inspection.

Product (Python, `crates/python/`): the hook finds fragments in Python
text from the tree-sitter tree of `py-syntax`:
- the **literal expression**: the largest expression made of a string
  literal, implicit concatenation, `+` of literals and expressions,
  parentheses, `"..." % args`, `"...".format(..)` and f-strings; each
  substitution is a hole; string prefixes, raw strings and triple quotes
  give the escapes. Sources: public behaviour of the reference and the
  Apache-2.0 `PyInjectionUtil`/`PyInjectorBase` of its Community code, read
  for the shape of this rule; a `notices/` entry is added if code is
  adapted.
- the **detection rule** that a literal is SQL; first match wins, an
  explicit "not SQL" always wins:
  1. a language comment before the string (`# language=SQL`,
     `# language=PostgreSQL`, `# language=Oracle`; `# language=none`
     turns it off) or the `Language("SQL")` annotation of the parameter or
     variable (typing metadata), which also names the dialect;
  2. an *Inject SQL* / *Don't inject* choice made by the user on this
     string with an intention, kept in the personal project state;
  3. the string is the first positional argument, or the `sql` / `query` /
     `statement` keyword argument, of a call whose callee name is in the
     configurable list (default: `execute`, `executemany`, `executescript`,
     `fetch`, `fetchrow`, `fetchval`, `fetchall`, `mogrify`, `text`,
     `read_sql`, `read_sql_query`, `read_sql_table`, `raw`, `RawSQL`,
     `SQL`) — matched by the last name of the callee, later refined by
     the type engine's knowledge of the receiver;
  4. a string whose first word is a SQL statement keyword (`select`,
     `insert`, `update`, `delete`, `with`, `merge`, `create`, `alter`,
     `drop`, `truncate`) followed in the same string by a structural
     keyword for it (`from`, `into`, `set`, `values`, `table`, `as`, ...)
     and that is at least two words long, assigned to a variable whose
     name is in a configurable pattern list (default: `sql`, `query`,
     `stmt`, `statement`, `*_sql`, `*_query`). This rule is on by default
     only with the variable-name condition; a setting extends it to every
     string that looks like SQL.
- the dialect and scope of the fragment are those of the Python file (the
  mapping, then the project default, ADR 0022 above); a `language=` comment
  overrides the dialect. Without a scope the fragment still gets syntax
  colouring, keyword completion and (strict) syntax errors.
- the injection list and name patterns are settings (`[python.sql-injection]`
  in `.oxide/project.toml`) and a Settings page.

Placeholders of database drivers (`%s`, `%(name)s`, `?`, `:name`, `$1`) are
lexed as SQL parameters in injected Python fragments: they are not holes
(the host does not substitute text), so such a fragment is strict.

### Performance
Initial targets, added to `docs/performance.md` and measured by the issue
that adds the code (results to `replica/perf.md`):

| Metric | Target |
|---|---|
| Lex and colour a 5000-line SQL script (about 250 KB) | <= 5 ms, off the UI thread; the UI thread only colours the visible lines |
| Re-lex after a keystroke | <= 0.5 ms (the lines touched, carried state) |
| Error-tolerant parse of a 5000-line script | <= 40 ms off the UI thread; re-parse of the edited statement <= 1 ms |
| SQL completion, request to items, script of 5000 lines bound to a model of 10 000 tables | p99 <= 30 ms (the popup budget of 100 ms after the answer stays) |
| Diagnostics (syntax and resolution) of the edited statement | <= 10 ms; of a 5000-line script on open <= 150 ms off the UI thread, statements in parallel |
| Format a 5000-line script / a 100-line statement | <= 200 ms / <= 5 ms |
| Memory of the trees of a 5000-line script | <= 30 MB |

Nothing of this runs on the UI thread except line colouring and applying
results. Requests are futures that are cancelled by dropping them; a newer
edit replaces older analysis. Large scripts are analysed statement by
statement, the statement under the caret first.

### Testing
- `sql-syntax`: unit and property tests (random token soup never panics;
  ranges are on character boundaries; the tree prints back to its input;
  re-parse after a random edit equals a full parse; the formatter keeps the
  token stream and is idempotent); corpora per dialect with the oracles
  above; PL/SQL and `$$` cases; every inspection and every grammar stage
  has fixtures with expected trees as text.
- `sql-intel`: scenarios on a fixture model (resolution, completion lists
  for the states `select c. from customers c`, joins with foreign keys,
  CTEs, quoted mixed-case names, schema-qualified names, `Unknown` versus
  `Unresolved`), quick fixes applied and re-checked, offset maps of
  injected fragments (escapes, holes, multi-line, non-ASCII), results
  mapped back.
- Platform: `RoutedCodeIntel` with fake providers (routing, merging,
  injection mapping); `check-layers.py` self-test.
- Product: Python detection fixtures, one per rule, positive and negative.
- UI: keystroke checks and scenarios in `tests/scenarios/db-gui/` and a
  new `tests/scenarios/sql/` for completion, quick fixes, formatting and
  injection, compared with reference screenshots where one exists.

## Consequences
- `crates/database/` is a new top-level directory; the crates of ADR 0021
  move there and `sql-text` becomes `sql-syntax`. Issues written against the
  old paths are amended. The workbench no longer composes the Database
  tool window: the product does, through the module.
- The workbench gets `Module`, `Product::with_module`, a keymap
  contribution mechanism and a `test-support` export of the keymap
  harness before the first UI slice of M6; `code-intel` gets
  `RoutedCodeIntel`, the injection model, `document_symbols` and
  `definition_targets`; `themes` gets the SQL text roles; `syntax` is
  unchanged (SQL does not use it).
- Own parser: more code than adopting a grammar, in exchange for correct
  trees on broken input and full control of dialects. The staging keeps
  every step shippable; the lexer-only fallback is the first stage.
- Python-side detection of SQL strings is heuristic and configurable; a
  false positive costs colour and possible inspections inside a string,
  so fragments without a scope report syntax errors only when strict, and
  everything can be turned off per string, per project and globally.
- Cross-file usages of database objects, a Problems tool window for SQL and
  database refactorings are not in the first version.
- Third-party material: the parser oracles are development-time
  dependencies (`sqlparser`, Apache-2.0; `libpg_query`, BSD-3-Clause) with
  notices if their text is adapted; datacraft's lexer and splitter
  (ADR 0021) remain the only adapted code for stage 1.
