use std::future::Future;
use std::time::Duration;

use crate::{Error, Result};

/// How long a statement may keep running after a cancel request before the
/// caller gives up on it and drops the future.
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Run `fut` with an optional time limit.
///
/// When the limit elapses, `on_timeout` runs (typically a server-side cancel)
/// and `fut` gets [`CANCEL_GRACE`] to finish. The result is then
/// `ErrorKind::Timeout`, unless `fut` succeeded inside the grace period. If it
/// is still running after the grace period it is dropped; drivers must treat
/// that as a broken connection (see the `Cursor` contract).
pub async fn with_timeout<T, F, C>(timeout: Option<Duration>, fut: F, on_timeout: C) -> Result<T>
where
    F: Future<Output = Result<T>>,
    C: Future<Output = ()>,
{
    let Some(limit) = timeout else {
        return fut.await;
    };
    tokio::pin!(fut);
    tokio::select! {
        r = &mut fut => return r,
        _ = tokio::time::sleep(limit) => {}
    }
    on_timeout.await;
    match tokio::time::timeout(CANCEL_GRACE, &mut fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) | Err(_) => Err(Error::timeout()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::ErrorKind;

    #[tokio::test]
    async fn no_limit_passes_through() {
        let r = with_timeout(None, async { Ok(7) }, async {}).await;
        assert_eq!(r.unwrap(), 7);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_runs_cancel_and_reports_timeout() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let mut tx = Some(tx);
        let slow = async move {
            // Ends with an error once the cancel arrives, like a server would.
            let _ = rx.await;
            Err::<(), _>(Error::cancelled())
        };
        let r = with_timeout(Some(Duration::from_millis(50)), slow, async move {
            flag.store(true, Ordering::SeqCst);
            tx.take().unwrap().send(()).ok();
        })
        .await;
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_grace() {
        let r: Result<()> = with_timeout(
            Some(Duration::from_millis(10)),
            std::future::pending(),
            async {},
        )
        .await;
        assert_eq!(r.unwrap_err().kind, ErrorKind::Timeout);
    }
}
