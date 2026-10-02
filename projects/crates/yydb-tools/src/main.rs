//! `yydb` command-line entrypoint (`yydb.exe` on Windows).

mod generator;
mod serve;

use std::{fs, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use yydb::Connection;

#[derive(Parser)]
#[command(
    name = "yydb",
    version = yydb::version(),
    about = "YYDB single-file database tools"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the library / CLI version.
    Version,
    /// Create or open a `.yydb` file and optionally set its schema.
    Init {
        /// Path to the database file.
        path: PathBuf,
        /// Schema version to store when the file has no schema yet.
        #[arg(long, default_value_t = 1)]
        schema_version: u32,
        /// Optional path to a VOS schema document.
        #[arg(long)]
        schema_file: Option<PathBuf>,
    },
    /// Print schema and record summary for a database file.
    Info {
        /// Path to the database file.
        path: PathBuf,
    },
    /// Serve a `.yydb` file for out-of-process / non-Rust clients (`yydb-client`).
    ///
    /// Binary protocol v1 over TCP and WebSocket (`/wire`). No ACL — default
    /// bind is loopback only. Do not expose on the public internet.
    Serve {
        /// Path to the database file.
        path: PathBuf,
        /// Bind address (host:port).
        #[arg(long, default_value = "127.0.0.1:7700")]
        bind: String,
        /// Allow non-loopback binds (dangerous; still not a public product).
        #[arg(long, default_value_t = false)]
        insecure_bind: bool,
    },
    /// Generate TypeScript types from a VOS schema (OXC AST codegen).
    Generate {
        /// Input `.vos` (or any text file containing VOS table decls).
        input: PathBuf,
        /// Output `.ts` path.
        #[arg(short, long)]
        output: PathBuf,
        /// Schema version embedded as `APP_SCHEMA_VERSION`.
        #[arg(long, default_value_t = 1)]
        schema_version: u32,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Version => {
            println!("{}", yydb::version());
        }
        Commands::Init {
            path,
            schema_version,
            schema_file,
        } => {
            let conn = Connection::open(&path)?;
            if let Some(schema_path) = schema_file {
                let document = fs::read_to_string(&schema_path)?;
                conn.ensure_schema(schema_version, &document)?;
            } else if conn.schema()?.is_none() {
                conn.ensure_schema(schema_version, "")?;
            }
            println!("initialized {}", path.display());
        }
        Commands::Info { path } => {
            let conn = Connection::open(&path)?;
            let path_display = conn
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<memory>".to_owned());
            println!("path: {path_display}");
            match conn.schema()? {
                Some(schema) => {
                    println!("schema.version: {}", schema.version);
                    let summary = summarize_document(&schema.document);
                    println!("schema.document: {summary}");
                }
                None => {
                    println!("schema: (none)");
                }
            }
            println!("records: {}", conn.record_count()?);
            println!("journal_mode: {}", conn.journal_mode().as_str());
            if let (Some(wal), Some(shm)) = (conn.wal_path(), conn.shm_path()) {
                let (wal_on, shm_on, frames) =
                    yydb::journal::sidecar_status(conn.path().expect("file-backed"))?;
                println!("wal: {} (exists={wal_on})", wal.display());
                println!("shm: {} (exists={shm_on})", shm.display());
                println!("wal_frames: {frames}");
            }
        }
        Commands::Serve {
            path,
            bind,
            insecure_bind,
        } => {
            serve::run_serve(&path, &bind, insecure_bind)?;
        }
        Commands::Generate {
            input,
            output,
            schema_version,
        } => {
            let source = fs::read_to_string(&input)?;
            let ts = generator::generate_typescript(&source, schema_version)?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&output, ts)?;
            println!("wrote {}", output.display());
        }
    }
    Ok(())
}

fn summarize_document(document: &str) -> String {
    let trimmed = document.trim();
    if trimmed.is_empty() {
        return "(empty)".to_owned();
    }
    const LIMIT: usize = 120;
    if trimmed.chars().count() <= LIMIT {
        return trimmed.replace('\n', " ");
    }
    let mut out: String = trimmed.chars().take(LIMIT).collect();
    out.push('…');
    out.replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::serve::assert_bind_allowed;

    #[test]
    fn loopback_bind_ok_without_insecure() {
        assert!(assert_bind_allowed("127.0.0.1:7700", false).is_ok());
        assert!(assert_bind_allowed("localhost:7700", false).is_ok());
    }

    #[test]
    fn non_loopback_rejected_without_insecure() {
        assert!(assert_bind_allowed("0.0.0.0:7700", false).is_err());
        assert!(assert_bind_allowed("192.168.1.10:7700", false).is_err());
    }

    #[test]
    fn non_loopback_allowed_with_insecure() {
        assert!(assert_bind_allowed("0.0.0.0:7700", true).is_ok());
    }
}
