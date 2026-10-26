//! The tokio runtime the interpreter work runs on: `py_env` starts its
//! processes with tokio (probing an interpreter, uv). A small runtime of
//! its own, created when first needed; the GUI threads never wait on it.

use std::future::Future;
use std::sync::OnceLock;

use gpui::{App, AppContext as _, Task};
use py_env::CancellationToken;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name(format!("{}-interpreters", brand::SLUG))
            .enable_all()
            .build()
            .expect("the interpreter runtime starts")
    })
}

/// Runs the future `work` makes on the runtime; dropping the task cancels
/// the token it was given (py-env stops its processes then).
pub(crate) fn spawn<F>(cx: &App, work: impl FnOnce(CancellationToken) -> F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let token = CancellationToken::new();
    let handle = runtime().spawn(work(token.clone()));
    let cancel = token.drop_guard();
    cx.background_spawn(async move {
        let _cancel = cancel;
        handle.await.expect("interpreter work does not panic")
    })
}
