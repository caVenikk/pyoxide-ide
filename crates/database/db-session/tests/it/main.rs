//! Sessions on real databases. SQLite runs in every test run; PostgreSQL
//! and Oracle tests are ignored and need the containers of
//! `docker-compose.yml` (see docs/testing-databases.md):
//!
//! ```text
//! docker compose up -d postgres oracle
//! cargo nextest run -p db-session --run-ignored all
//! ```
//!
//! Hosts, ports and the password come from `PYOXIDE_TEST_PG_HOST`,
//! `PYOXIDE_TEST_PG_PORT` (default `55432`), `PYOXIDE_TEST_ORACLE_HOST`,
//! `PYOXIDE_TEST_ORACLE_PORT` (default `1521`),
//! `PYOXIDE_TEST_ORACLE_SERVICE` (default `FREEPDB1`) and
//! `PYOXIDE_DB_PASSWORD` (default `pyoxide`).

mod oracle;
mod pager;
mod postgres;
mod refresh;
mod sqlite;
mod support;
