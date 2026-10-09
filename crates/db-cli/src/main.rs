//! `pyoxide-db`: first vertical slice of the Pyoxide IDE database client.

use clap::{Parser, Subcommand, ValueEnum};
use db_core::{
    Column, ConnectParams, Connection, ResultSet, Secret, Value, ValueKind, render_table, schema,
};

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
        database: cli.service.clone(),
        user: cli.user.clone(),
        password: Secret::new(cli.password.clone()),
        ..Default::default()
    };
    let backend = match cli.backend {
        Backend::Thin => db_core::Backend::OracleThin,
        Backend::Odpi => db_core::Backend::OracleOdpi,
    };
    db_service::driver(backend).connect(&p).await
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
    let rs = match &cli.cmd {
        Cmd::Ping => {
            conn.query("SELECT 'pong' AS reply FROM dual", &[], 1)
                .await?
        }
        Cmd::Query { sql } => conn.query(sql, &[], cli.max_rows).await?,
        Cmd::Tables => {
            let intro = schema::introspector(conn.as_ref());
            let owner = intro.current_schema().await?;
            let objects = intro.objects(&owner, &[schema::ObjectKind::Table]).await?;
            text_table(
                &["OWNER", "TABLE_NAME"],
                objects
                    .into_iter()
                    .map(|o| vec![o.schema, o.name])
                    .collect(),
            )
        }
        Cmd::Describe { owner, table } => {
            let intro = schema::introspector(conn.as_ref());
            let cols = intro.columns(owner, table).await?;
            text_table(
                &["COLUMN_NAME", "DATA_TYPE", "NULLABLE"],
                cols.into_iter()
                    .map(|c| {
                        vec![
                            c.name,
                            c.data_type,
                            if c.nullable { "Y" } else { "N" }.into(),
                        ]
                    })
                    .collect(),
            )
        }
    };
    println!("[{}]", conn.backend());
    println!("{}", render_table(&rs));
    conn.close().await.ok();
    Ok(())
}

fn text_table(names: &[&str], rows: Vec<Vec<String>>) -> ResultSet {
    ResultSet {
        columns: names
            .iter()
            .map(|n| Column {
                name: (*n).into(),
                type_name: "TEXT".into(),
                kind: ValueKind::Text,
                nullable: true,
            })
            .collect(),
        rows: rows
            .into_iter()
            .map(|r| r.into_iter().map(Value::Text).collect())
            .collect(),
        ..Default::default()
    }
}
