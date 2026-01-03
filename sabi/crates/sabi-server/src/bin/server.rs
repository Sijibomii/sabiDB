//! SabiDB server binary

use std::sync::Arc;
use clap::Parser;
use tracing_subscriber;
use sabi_server::SabiServer;
use sabi_storage::{StorageEngine, PageFile, WalWriter};
use sabi_sql::Catalog;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Database file path
    #[arg(short, long, default_value = "sabidb.data")]
    data_file: String,
    
    /// WAL file path
    #[arg(short, long, default_value = "sabidb.wal")]
    wal_file: String,
    
    /// Server address
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    addr: String,
    
    /// Enable debug logging
    #[arg(short, long)]
    verbose: bool,
    
    /// Initial catalog schema file (optional)
    #[arg(short, long)]
    schema: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Initialize logging
    let log_level = if args.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .init();
    
    println!("Starting SabiDB Server v{}", env!("CARGO_PKG_VERSION"));
    println!("Data file: {}", args.data_file);
    println!("WAL file: {}", args.wal_file);
    println!("Listening on: {}", args.addr);
    
    // Initialize storage
    let pages = PageFile::open(&args.data_file)?;
    let wal = WalWriter::new(&args.wal_file)?;
    let storage = Arc::new(StorageEngine::new(pages, wal)?);
    
    // Initialize catalog (load schema if provided)
    let catalog = if let Some(schema_path) = args.schema {
        Catalog::load_from_file(&schema_path)?
    } else {
        Catalog::new()
    };
    
    // Create and start server
    let server = Arc::new(SabiServer::new(storage, catalog).await?);
    server.start(&args.addr).await?;
    
    Ok(())
}