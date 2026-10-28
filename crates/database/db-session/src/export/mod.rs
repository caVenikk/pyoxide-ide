//! Export of a table or a query result to a file or to memory (ADR 0021:
//! "Export and import are jobs too"). The job runs the query again without
//! paging on a data connection, reads it a fetch at a time and hands every
//! row to a [`writers::RowWriter`], so memory stays flat whatever the size
//! of the result. A file is written under a temporary name next to its
//! destination and renamed when the export is complete: a cancelled or
//! failed export leaves no file and a finished one is never half written.

mod format;
mod job;
pub mod writers;

pub use format::{ExportOptions, Format};
pub use job::{
    ExportDone, ExportEvent, ExportOutcome, ExportRequest, ExportSink, ExportSource, ExportTarget,
};

#[cfg(test)]
mod tests;
