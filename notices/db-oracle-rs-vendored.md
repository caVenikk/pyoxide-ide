- vendor/oracle-rs (whole crate)
  Copied from oracle-rs 0.1.7 (MIT OR Apache-2.0),
  https://github.com/stiang/oracle-rs, the package published on crates.io
  as version 0.1.7, with its LICENSE-MIT and LICENSE-APACHE files.
  Changes: fixes listed in vendor/oracle-rs/PYOXIDE-CHANGES.md (rows of a
  fetch keep the previous row's values across round trips, #334, and other
  fixes taken from upstream issues, pull requests and forks); the crate is
  used through `[patch.crates-io]` in the workspace Cargo.toml.
  Code adapted from forks of oracle-rs (each fork keeps the upstream licence
  MIT OR Apache-2.0 and its LICENSE files; we use it under Apache-2.0):
  - https://github.com/appcard/oracle-rs, commits 6c4d64def7a013d2d96484a1743d6a946ff49168
    and 2c3556e99c4ce20a94e51224947b9430d5b673f5 (src/types/date.rs only),
    Oracle 19c error block, marker packets with
    a four-byte length, has_more_rows, fetch request framing, replies that
    span packets, NULL literal columns, ORA code shown once, named time zone.
  - https://github.com/Keeper-Security/oracle-rs-fork, commits
    3abfbd3a7d4526723afdb2a904081069b3ebdfe5,
    8c7bf3d7e13cb8d8b6a3b29bbc6cb27e1d874148,
    ae99d374c0baa74fc9faffed7242c197e3e1df50 and
    a17e65dfe21ee8f4114ce121c5dbfdcc72d758da: server-side piggyback and warning messages are skipped,
    LONG and LONG RAW columns are fetched with a define and their two-field
    trailer is read.
