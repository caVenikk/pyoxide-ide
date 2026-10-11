//! The database service behind the `db.*` commands, set up the way
//! `pyoxide-db` does it.

use std::path::PathBuf;
use std::sync::Arc;

use db_service::DbService;
use secrets::terminal::TerminalPrompter;

pub fn service(profiles_path: Option<PathBuf>) -> DbService {
    DbService::configured(profiles_path, Some(Arc::new(TerminalPrompter)))
}
