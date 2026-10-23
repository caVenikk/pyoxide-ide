# 0005. Platform core and language products in one repository

- Status: accepted
- Date: 2026-10-11

## Context
Most of an IDE does not depend on the language: editor, UI shell, command
registry, project model, version control, terminal, LSP and DAP clients, the
database client, secrets, themes and settings import. Language support is
mostly configuration and tooling integration on top of LSP/DAP: interpreters
and environments, build and test runners, debugger launch settings. Separate
lean IDEs per language (instead of one universal IDE with plugins) keep each
product fast and simple, but should share that core.

## Options considered
1. One universal IDE with runtime plugins per language. Every product carries
   every subsystem; startup and memory suffer; contradicts the project goals.
2. Separate repositories for the platform and for each product. Clear
   ownership, but while internal APIs change weekly every cross-cutting change
   needs a platform release, a version bump and a second pull request.
3. One repository and Cargo workspace with a platform layer and product
   layers, composed at compile time into one binary per product.

## Decision
Option 3.

- `crates/platform/`: crates that know nothing about a specific language.
- `crates/<product>/` (first: `crates/python/`): language crates and the
  product binary.
- Platform crates never depend on product crates. `scripts/check-layers.py`
  enforces this in CI from `cargo metadata`.
- Products are composed at compile time (crate dependencies and Cargo
  features), not through runtime plugins. The WASM plugin API (M7) is for
  third-party extensions, not for the products themselves.
- Extension points for languages (for example a `LanguageSupport` trait) are
  introduced when a second product starts, from what the first one actually
  needed, not designed ahead.

The platform moves to its own repository only when there is a reason: a
second product with its own contributors or release cadence, and stable
platform APIs. The layer boundary keeps that split mechanical (history can be
extracted with `git filter-repo`; products then depend on tagged platform
releases).

## Consequences
- Cross-cutting changes stay atomic: one commit, one CI run, one lock file.
- Each product binary contains only its own language support.
- New code goes to the platform unless it is language-specific; reviewers
  check placement.
- The repository may be renamed once the platform has a name; GitHub keeps
  redirects for the old URL.

## Update: modules (ADR 0022)
Between the platform and the products there is a third kind of layer, a
**module**: a reusable set of crates in its own directory of `crates/`
(first: `crates/database/`) that several products can include. A module
depends on platform crates only, products depend on platform crates and
modules, the platform never depends on a module, and a module never depends
on a product or on another module. `scripts/check-layers.py` enforces it.
The language extension point this ADR deferred (routing a document to the
code intelligence of its language) arrived with the second language, SQL;
it is specified in ADR 0022.
