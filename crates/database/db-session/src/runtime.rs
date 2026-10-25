use std::future::Future;
use std::sync::Arc;

use db_core::{ConnectionSource, DataSourceId};

use crate::{DataSourceHandle, DataSourceOptions, EventSink};

/// The database runtime (ADR 0021): a small multi-thread async runtime with
/// named threads on which every connection, session and job of the database
/// module runs. The product starts one and hands it to the module; the UI
/// never awaits anything of it, it calls methods that return at once and
/// receives [`Event`](crate::Event)s through an [`EventSink`].
///
/// Cheap to clone; the threads stop when the last clone is dropped (work
/// still running is abandoned, connections are dropped).
#[derive(Clone)]
pub struct DbRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: Option<tokio::runtime::Runtime>,
    handle: tokio::runtime::Handle,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Never blocks. The last clone may be dropped by a task of this very
        // runtime (a session ending) or inside another runtime: shut down
        // from a thread of its own then.
        if let Some(rt) = self.runtime.take() {
            if tokio::runtime::Handle::try_current().is_ok() {
                let _ = std::thread::Builder::new()
                    .name("pyoxide-db-shutdown".into())
                    .spawn(move || rt.shutdown_background());
            } else {
                rt.shutdown_background();
            }
        }
    }
}

impl std::fmt::Debug for DbRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbRuntime")
    }
}

impl DbRuntime {
    /// Two worker threads named `pyoxide-db`: enough for many idle sessions,
    /// since drivers with blocking client libraries run on threads of their
    /// own (`db_core::worker`).
    pub fn new() -> std::io::Result<Self> {
        Self::with_threads(2)
    }

    pub fn with_threads(threads: usize) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads.max(1))
            .thread_name("pyoxide-db")
            .enable_all()
            .build()?;
        let handle = runtime.handle().clone();
        Ok(Self {
            inner: Arc::new(Inner {
                runtime: Some(runtime),
                handle,
            }),
        })
    }

    /// A handle for one data source. Nothing connects until a session runs
    /// something, [`DataSourceHandle::connect`] or a refresh is called.
    pub fn data_source(
        &self,
        id: DataSourceId,
        options: DataSourceOptions,
        source: Arc<dyn ConnectionSource>,
        sink: EventSink,
    ) -> DataSourceHandle {
        DataSourceHandle::new(self.clone(), id, options, source, sink)
    }

    pub(crate) fn spawn<F>(&self, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.inner.handle.spawn(fut);
    }
}
