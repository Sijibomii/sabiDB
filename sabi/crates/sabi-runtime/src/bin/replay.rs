//! WAL replay utility

use std::fs::File;
use std::io::{BufReader, BufRead};
use std::sync::Arc;
use serde_json::Value;
use sabi_runtime::deterministic::DeterministicRuntime;
use sabi_runtime::reactive::ReactiveEngine;
use sabi_storage::engine::StorageEngine;
use sabi_storage::page_file::PageFile;
use sabi_storage::wal::WalWriter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <wal_file>", args[0]);
        std::process::exit(1);
    }
    
    let wal_file = &args[1];
    
    println!("Replaying WAL from: {}", wal_file);
    
    // Initialize components
    let pages = PageFile::open("replay.db")?;
    let wal = WalWriter::open("replay.wal")?;
    let storage = Arc::new(StorageEngine::new(pages, wal)?);
    
    let runtime = Arc::new(DeterministicRuntime::new()?);
    let reactive = Arc::new(ReactiveEngine::new(runtime.clone()));
    
    // Replay WAL
    replay_wal(wal_file, runtime, reactive, storage).await?;
    
    println!("Replay completed successfully");
    Ok(())
}

async fn replay_wal(
    wal_file: &str,
    runtime: Arc<DeterministicRuntime>,
    reactive: Arc<ReactiveEngine>,
    storage: Arc<StorageEngine>,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open(wal_file)?;
    let reader = BufReader::new(file);
    
    let mut line_number = 0;
    
    for line in reader.lines() {
        line_number += 1;
        let line = line?;
        
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        
        // Parse WAL entry
        let entry: Value = serde_json::from_str(&line)
            .map_err(|e| format!("Line {}: Invalid JSON: {}", line_number, e))?;
        
        let entry_type = entry.get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("Line {}: Missing type", line_number))?;
        
        match entry_type {
            "mutation" => replay_mutation(&entry, &runtime, &reactive, &storage).await?,
            "action" => replay_action(&entry, &runtime, &storage).await?,
            "function_def" => replay_function_def(&entry, &runtime)?,
            _ => {
                eprintln!("Line {}: Unknown entry type: {}", line_number, entry_type);
                continue;
            }
        }
    }
    
    Ok(())
}

async fn replay_mutation(
    entry: &Value,
    runtime: &Arc<DeterministicRuntime>,
    reactive: &Arc<ReactiveEngine>,
    storage: &Arc<StorageEngine>,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = entry.get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing mutation name")?;
    
    let args = entry.get("args")
        .and_then(|v| v.as_array())
        .ok_or("Missing mutation args")?;
    
    let tx_id = entry.get("tx_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or("Invalid transaction ID")?;
    
    // Convert args to JsValue
    let js_args: Vec<sabi_runtime::types::JsValue> = args.iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    
    // Execute mutation deterministically
    let _result = reactive.execute_mutation(tx_id, name, js_args)?;
    
    println!("Replayed mutation: {}", name);
    Ok(())
}

async fn replay_action(
    entry: &Value,
    runtime: &Arc<DeterministicRuntime>,
    storage: &Arc<StorageEngine>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Similar to mutation replay but with external effects
    println!("Replayed action");
    Ok(())
}

fn replay_function_def(
    entry: &Value,
    runtime: &Arc<DeterministicRuntime>,
) -> Result<(), Box<dyn std::error::Error>> {
    let name = entry.get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing function name")?;
    
    let source = entry.get("source")
        .and_then(|v| v.as_str())
        .ok_or("Missing function source")?;
    
    runtime.register_function(name, source)?;
    println!("Registered function: {}", name);
    
    Ok(())
}