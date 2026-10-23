- crates/database/sql-syntax/src/lexer.rs, split.rs, classify.rs (the block
  recognition), explain.rs, tests/it/split.rs, tests/it/lexer.rs
  Adapted from datacraft (MIT OR Apache-2.0),
  https://github.com/javy-fernandez/datacraft @
  8beb347c40e47c8107223e783b6e1be80c798a31, file src/sql.rs: the tokenizer
  (string, quoted identifier and comment forms, `$tag$` quotes, Oracle
  `q'[..]'` literals, the subscript-or-bracket-name rule, byte-level
  scanning with boundaries on characters), the statement splitter (the
  procedural block tracker for PL/SQL and trigger bodies, the recognition of
  statements that start a block, the `/` line terminator, the PL/SQL
  `END` handling), the rule that picks the statement at a caret, the shape
  of the explain wrapper, and the test cases for them.
  Changes: only PostgreSQL, Oracle, SQLite and a generic dialect are kept
  (no SQL Server, MySQL or Snowflake rules); dialect rules are data
  (`LexRules`) instead of per-driver flags; the lexer keeps every byte
  (whitespace and comments are tokens of their own, so concatenating the
  tokens gives the input back), has parameter, operator, hole and
  `SlashLine` tokens, unterminated flags and trivia attachment; the splitter
  also handles PostgreSQL `BEGIN ATOMIC` bodies and `<<label>>` blocks and
  returns a classification (kind, data-changing, `DELETE`/`UPDATE` without
  `WHERE`, DDL target names); statement lookup by caret, candidates and
  selections, the parameter scan, the keyword tables and the identifier
  rules are new. None of its interface code, themes, icons, strings or
  assets is used.
