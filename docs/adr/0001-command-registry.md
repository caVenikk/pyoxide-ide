# 0001. Command registry

- Status: accepted
- Date: 2026-10-09

## Context
Every user-visible action must be reachable from the UI, the CLI and an agent
control channel with one implementation. Without a shared entry point, the CLI
and the UI drift apart and agents need special code paths.

## Options considered
1. Ad-hoc functions per front-end. Simple, but duplicates argument handling.
2. A typed trait per command with an associated `Args`/`Output` type. Strong
   typing, but a heterogeneous registry needs type erasure anyway.
3. A registry of type-erased commands with JSON arguments and results, plus an
   adapter that wraps typed async functions.

## Decision
Option 3, in the `commands` crate:

- `CommandMeta { id, title, summary, args_example }`; ids are stable and
  dotted (`db.query`, `db.profile.list`).
- `Command::invoke(serde_json::Value) -> Result<Value, CommandError>`.
- `typed(meta, async fn(Args) -> Result<Output>)` builds a command from a
  typed function; Rust callers may call the typed function directly, the JSON
  path calls the same function.
- `CommandError { code, message }`: codes are stable (`invalid_args`,
  `not_found`, `db.auth`, ...), messages never contain secrets.
- Front-ends: the CLI maps sub-commands to typed calls and exposes
  `pyoxide-db call <id> <json>` and `pyoxide-db commands`; the UI command
  palette and the control channel read the same registry.

## Consequences
- New features register commands first; widgets hold no logic.
- JSON is the wire format of the control channel, so result types derive
  `Serialize`. Secrets are never part of command arguments or results.
- Long-running or streaming commands (data grid paging) need session state;
  that is added with the application shell, on top of this contract.
