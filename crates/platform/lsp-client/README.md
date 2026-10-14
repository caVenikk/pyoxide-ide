# lsp-client

The Language Server Protocol client of Pyoxide IDE (ADR 0010):
`LspCodeIntel` implements `code_intel::CodeIntel` over the language servers a
product describes with `ServerSpec`s.

## Tests

`cargo test -p lsp-client` runs the unit tests and the tests against
`lsp-fake-server`, a scripted server built with the crate
(`src/bin/lsp-fake-server.rs`; its script format is described there).

`tests/real_servers.rs` runs against real servers and is ignored by default.
It needs:

| server | executable | variable |
|---|---|---|
| ruff | `ruff` | `LSP_CLIENT_RUFF` |
| ty | `ty` | `LSP_CLIENT_TY` |
| basedpyright | `basedpyright-langserver` | `LSP_CLIENT_BASEDPYRIGHT` |

Each test takes the variable, else looks the executable up on `PATH`. To get
ty and basedpyright without touching your own tools, install them into a
temporary folder with uv:

```sh
export UV_TOOL_DIR=/tmp/lsp-tools UV_TOOL_BIN_DIR=/tmp/lsp-tools/bin
uv tool install ty
uv tool install basedpyright
export LSP_CLIENT_TY=/tmp/lsp-tools/bin/ty
export LSP_CLIENT_BASEDPYRIGHT=/tmp/lsp-tools/bin/basedpyright-langserver
export LSP_CLIENT_RUFF="$(command -v ruff)"
cargo test -p lsp-client --test real_servers -- --ignored
```

Run one server's test by name, for example `-- --ignored ruff`.
