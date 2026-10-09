# Clean-room rules

Allowed sources:
1. Public documentation, specifications and protocols (LSP, DAP, Oracle data
   dictionary docs, Docker Engine API, Python docs).
2. Open-source code under compatible licenses (Apache-2.0, MIT, BSD), with an
   entry in `NOTICE` for anything copied or closely adapted.
3. **Observed behaviour** of tools a contributor is licensed to use: run a
   scenario, record inputs and outputs, write them as tests under
   `tests/scenarios/`. Never copy text, icons, layout assets or strings.
4. Users' own configuration files (exported keymaps, colour schemes, live
   templates) as an import format. User themes and colour schemes (`.icls` XML,
   `.theme.json`, keymap and code-style XML) are plain data: the importer is
   written from documented formats and sample files. Plugin archives
   (`.jar`/zip) may be opened **only to extract such data resources**;
   `.class` files and anything executable are never opened, decompiled or analysed.

Forbidden:
- Decompiled, disassembled or otherwise extracted code from proprietary
  products, in any form (including "translated" or "paraphrased").
- Proprietary assets, trademarks, logos, strings or sound.
- Pasting such material into prompts, issues, commits or docs.
- Committing third-party theme files whose license does not allow it. Keep them
  in the git-ignored `tests/fixtures/themes-local/`.

When behaviour is unclear, write the *question* and an observation script
(scenario -> expected output) in `tests/scenarios/`, never the internals of
the other tool.
