//! The cancel request, sent over a short-lived second connection.
//!
//! tokio-postgres returns from a cancel as soon as the request is written. The
//! server acts on it a little later, so a statement started right after the
//! cancel can be hit by it. This module waits until the server has processed
//! the request and closed the connection (like libpq does), which makes
//! "cancel, then run the next statement" safe.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use db_core::timeout::CANCEL_GRACE;
use db_core::{Error, ErrorKind, Result};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_postgres::CancelToken;
use tokio_postgres::tls::MakeTlsConnect;
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::error::{Phase, map_scrubbed};

/// Where the server listens (a host name, an address or a socket directory).
#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub host: String,
    pub port: u16,
}

/// Send the cancel request and wait for the server to be done with it.
pub(crate) async fn request(
    token: &CancelToken,
    tls: &MakeRustlsConnect,
    target: &Target,
    secret: &str,
) -> Result<()> {
    let run = async {
        #[cfg(unix)]
        if target.host.starts_with('/') {
            let path = format!(
                "{}/.s.PGSQL.{}",
                target.host.trim_end_matches('/'),
                target.port
            );
            let stream = tokio::net::UnixStream::connect(path)
                .await
                .map_err(network_error)?;
            return send(token, tls, target, stream, secret).await;
        }
        let stream = TcpStream::connect((target.host.as_str(), target.port))
            .await
            .map_err(network_error)?;
        let _ = stream.set_nodelay(true);
        send(token, tls, target, stream, secret).await
    };
    tokio::time::timeout(CANCEL_GRACE, run)
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(
                ErrorKind::Network,
                "the cancel request timed out",
            ))
        })
}

fn network_error(e: io::Error) -> Error {
    Error::new(
        ErrorKind::Network,
        format!("cannot send the cancel request: {e}"),
    )
}

async fn send<S>(
    token: &CancelToken,
    tls: &MakeRustlsConnect,
    target: &Target,
    stream: S,
    secret: &str,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut tls = tls.clone();
    let connect = MakeTlsConnect::<WaitForClose<S>>::make_tls_connect(&mut tls, &target.host)
        .map_err(|e| match e {})?;
    token
        .cancel_query_raw(WaitForClose(stream), connect)
        .await
        .map_err(|e| map_scrubbed(&e, Phase::Session, secret))
}

/// A stream whose `shutdown` also waits for the peer to close its side.
struct WaitForClose<S>(S);

impl<S: AsyncRead + Unpin> AsyncRead for WaitForClose<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WaitForClose<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(Pin::new(&mut self.0).poll_shutdown(cx))?;
        let mut storage = [0u8; 64];
        let mut buf = ReadBuf::new(&mut storage);
        loop {
            match Pin::new(&mut self.0).poll_read(cx, &mut buf) {
                Poll::Ready(Ok(())) if buf.filled().is_empty() => return Poll::Ready(Ok(())),
                Poll::Ready(Ok(())) => buf.clear(),
                // A reset also means the server is done with the request.
                Poll::Ready(Err(_)) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn shutdown_waits_until_the_peer_closes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            socket.read_to_end(&mut received).await.unwrap();
            // keep the connection open a little before closing it
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            received
        });

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut stream = WaitForClose(stream);
        stream.write_all(b"request").await.unwrap();
        let started = std::time::Instant::now();
        stream.shutdown().await.unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(150));
        assert_eq!(server.await.unwrap(), b"request");
    }
}
