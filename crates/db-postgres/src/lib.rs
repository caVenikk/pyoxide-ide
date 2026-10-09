//! PostgreSQL backend in pure Rust (tokio-postgres, rustls with ring).
//!
//! [`PostgresDriver`] opens [`PostgresConnection`]s that implement the
//! `db-core` contract: typed binary decoding, true incremental paging with a
//! streaming cursor, server-side cancel, per-call timeouts and transaction
//! modes.

mod bind;
mod cancel;
mod conn;
mod cursor;
mod decode;
mod error;
mod interval;
mod numeric;
mod stmt;
mod tls;

use std::time::Duration;

use async_trait::async_trait;
use db_core::{Backend, ConnectParams, Connection, Driver, Error, ErrorKind, Result, opt};

pub use conn::PostgresConnection;

const DEFAULT_APPLICATION_NAME: &str = "pyoxide";

pub struct PostgresDriver;

#[async_trait]
impl Driver for PostgresDriver {
    fn backend(&self) -> Backend {
        Backend::Postgres
    }

    fn availability(&self) -> Result<String> {
        Ok("built-in (tokio-postgres)".into())
    }

    async fn connect(&self, params: &ConnectParams) -> Result<Box<dyn Connection>> {
        let secret = params.password.expose();
        let tls = tls::connector(&params.tls).map_err(|e| e.scrub(secret))?;

        // `[::1]` is the URL spelling of an IPv6 address; the socket layer
        // wants the bare address.
        let host = match params.host.trim_matches(['[', ']']) {
            "" => "localhost",
            host => host,
        };
        let port = if params.port == 0 {
            Backend::Postgres.default_port().unwrap_or(5432)
        } else {
            params.port
        };

        let mut config = tokio_postgres::Config::new();
        config
            .host(host)
            .port(port)
            .user(&params.user)
            .application_name(
                params
                    .option(opt::APPLICATION_NAME)
                    .unwrap_or(DEFAULT_APPLICATION_NAME),
            )
            .ssl_mode(tls::ssl_mode(params.tls.mode))
            .keepalives_idle(Duration::from_secs(60));
        if !params.database.is_empty() {
            config.dbname(&params.database);
        }
        if !params.password.is_empty() {
            config.password(secret);
        }
        if !params.connect_timeout.is_zero() {
            config.connect_timeout(params.connect_timeout);
        }

        let connecting = config.connect(tls.clone());
        let (client, connection) = if params.connect_timeout.is_zero() {
            connecting.await
        } else {
            // `connect_timeout` only covers the TCP connect; this also bounds
            // the TLS handshake and authentication.
            tokio::time::timeout(params.connect_timeout, connecting)
                .await
                .map_err(|_| {
                    Error::new(
                        ErrorKind::Network,
                        format!(
                            "connecting to {host}:{port} timed out after {:?}",
                            params.connect_timeout
                        ),
                    )
                })?
        }
        .map_err(|e| error::map_scrubbed(&e, error::Phase::Connect, secret))?;

        let task = tokio::spawn(async move {
            let _ = connection.await;
        });

        if params.read_only
            && let Err(e) = client
                .batch_execute("SET default_transaction_read_only = on")
                .await
        {
            task.abort();
            return Err(error::map_scrubbed(&e, error::Phase::Connect, secret));
        }

        let target = cancel::Target {
            host: host.to_owned(),
            port,
        };
        let inner = conn::Inner::new(client, task, tls, params.password.clone(), target);
        Ok(Box::new(PostgresConnection::new(inner)))
    }
}

#[cfg(test)]
mod tests {
    use db_core::Secret;

    use super::*;

    #[test]
    fn driver_identity() {
        assert_eq!(PostgresDriver.backend(), Backend::Postgres);
        assert!(
            PostgresDriver
                .availability()
                .unwrap()
                .contains("tokio-postgres")
        );
    }

    #[tokio::test]
    async fn bad_tls_setup_is_reported_before_connecting() {
        let params = ConnectParams {
            host: "127.0.0.1".into(),
            port: 1,
            password: Secret::new("hunter2"),
            tls: db_core::TlsParams {
                mode: db_core::TlsMode::VerifyFull,
                ca_file: Some("/nonexistent/ca.pem".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let e = PostgresDriver.connect(&params).await.err().unwrap();
        assert_eq!(e.kind, ErrorKind::Config);
        assert!(!e.message.contains("hunter2"));
    }
}
