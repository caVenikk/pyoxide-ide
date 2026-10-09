//! `pyoxide-db`: first vertical slice of the Pyoxide IDE database client.

use clap::{Parser, Subcommand, ValueEnum};
use db_core::{ConnectParams, Connection, Driver, introspection, render_table};

#[derive(Copy, Clone, ValueEnum)]
enum Backend {
    /// Pure Rust, no Oracle Client needed.
    Thin,
    /// Uses an installed Oracle Client (ODPI-C).
    Odpi,
}

#[derive(Parser)]
#[command(
    name = "pyoxide-db",
    version,
    about = "Oracle client (CLI slice of Pyoxide IDE)"
)]
struct Cli {
    #[arg(long, value_enum, default_value = "thin")]
    backend: Backend,
    #[arg(long, env = "PYOXIDE_DB_HOST", default_value = "localhost")]
    host: String,
    #[arg(long, env = "PYOXIDE_DB_PORT", default_value_t = 1521)]
    port: u16,
    #[arg(long, env = "PYOXIDE_DB_SERVICE")]
    service: String,
    #[arg(long, env = "PYOXIDE_DB_USER")]
    user: String,
    /// Read from the environment only, so it never lands in shell history.
    #[arg(skip)]
    password: String,
    /// Max rows to fetch per query.
    #[arg(long, default_value_t = 500)]
    max_rows: usize,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check the connection and print server round-trip result.
    Ping,
    /// Run one SQL statement.
    Query { sql: String },
    /// List user tables.
    Tables,
    /// Describe columns of OWNER.TABLE.
    Describe { owner: String, table: String },
}

async fn connect(cli: &Cli) -> db_core::Result<Box<dyn Connection>> {
    let p = ConnectParams {
        host: cli.host.clone(),
        port: cli.port,
        service: cli.service.clone(),
        user: cli.user.clone(),
        password: cli.password.clone(),
    };
    match cli.backend {
        Backend::Thin => db_oracle_thin::ThinDriver::connect(&p).await,
        Backend::Odpi => db_oracle_odpi::OdpiDriver::connect(&p).await,
    }
}

#[tokio::main]
async fn main() {
    let mut cli = Cli::parse();
    cli.password = std::env::var("PYOXIDE_DB_PASSWORD").unwrap_or_default();
    if cli.password.is_empty() {
        eprintln!("error: set PYOXIDE_DB_PASSWORD in the environment");
        std::process::exit(2);
    }
    if let Err(e) = run(&cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: &Cli) -> db_core::Result<()> {
    let conn = connect(cli).await?;
    let sql = match &cli.cmd {
        Cmd::Ping => "SELECT 'pong' AS reply FROM dual".to_string(),
        Cmd::Query { sql } => sql.clone(),
        Cmd::Tables => introspection::ORACLE_LIST_TABLES.to_string(),
        Cmd::Describe { owner, table } => introspection::oracle_describe_table(owner, table)
            .ok_or_else(|| db_core::Error::Query("invalid owner/table name".into()))?,
    };
    let rs = conn.query(&sql, cli.max_rows).await?;
    println!("[{}]", conn.backend());
    println!("{}", render_table(&rs));
    conn.close().await.ok();
    Ok(())
}
