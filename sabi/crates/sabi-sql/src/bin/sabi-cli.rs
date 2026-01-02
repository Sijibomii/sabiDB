//! Command-line interface for SabiDB

use std::sync::Arc;
use std::sync::Mutex;

use rustyline::{Editor, error::ReadlineError};
use rustyline::history::DefaultHistory;

use sabi_storage::page_file::PageFile;
use sabi_storage::engine::{StorageEngine};
use sabi_storage::wal::WalWriter;

use sabi_sql::parser::QueryParser;
use sabi_sql::planner::{QueryPlanner, Catalog};
use sabi_sql::executor::QueryExecutor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("SabiDB - A Simple MVCC Database");
    println!("Version 0.1.0");
    println!("Type 'help' for help, 'exit' to quit\n");
    
    // Initialize storage
    let pages = PageFile::open("sabidb.data")?;
    let wal = WalWriter::open("sabidb.wal")?;
    let storage = Arc::new(Mutex::new(StorageEngine::new(pages, wal)?));
    
    // Initialize SQL components
    let parser = QueryParser::new();
    let catalog = Catalog::new();
    let planner = QueryPlanner::new(catalog);
    let mut executor = QueryExecutor::new(storage.clone(), planner.catalog.clone());
    
    // Setup readline with history
    let mut rl = Editor::<(), DefaultHistory>::new()?;
    if rl.load_history("sabidb.history").is_err() {
        println!("No history found, starting fresh");
    }
    
    loop {
        let readline = rl.readline("sabi> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str())?;
                
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                
                match trimmed.to_lowercase().as_str() {
                    "exit" | "quit" => break,
                    "help" => print_help(),
                    _ => execute_query(trimmed, &parser, &planner, &mut executor)?,
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
    
    rl.save_history("sabidb.history")?;
    println!("Goodbye!");
    Ok(())
}

fn execute_query(
    sql: &str,
    parser: &QueryParser,
    planner: &QueryPlanner,
    executor: &mut QueryExecutor,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse SQL
    let statements = parser.parse(sql)
        .map_err(|e| format!("Parse error: {}", e))?;
    
    for stmt in statements {
        // Plan statement
        let plan = planner.plan(stmt)
            .map_err(|e| format!("Plan error: {}", e))?;
        
        // Execute plan
        match executor.execute(plan) {
            Ok(result) => {
                match result {
                    sabi_sql::executor::QueryResult::CreateTable => {
                        println!("Table created");
                    }
                    sabi_sql::executor::QueryResult::Insert(count) => {
                        println!("Inserted {} row(s)", count);
                    }
                    sabi_sql::executor::QueryResult::Select { columns, rows } => {
                        // Print header
                        println!("{}", columns.join(" | "));
                        println!("{}", "-".repeat(columns.join(" | ").len()));
                        
                        // Print rows
                        for row in rows.clone() {
                            let formatted: Vec<String> = row.iter()
                                .map(|v| v.to_string())
                                .collect();
                            println!("{}", formatted.join(" | "));
                        }
                        println!("{} row(s) returned", rows.len());
                    }
                    sabi_sql::executor::QueryResult::Delete(count) => {
                        println!("Deleted {} row(s)", count);
                    }
                    sabi_sql::executor::QueryResult::BeginTransaction(tx_id) => {
                        println!("Transaction started: {}", tx_id);
                    }
                    sabi_sql::executor::QueryResult::Commit => {
                        println!("Transaction committed");
                    }
                    sabi_sql::executor::QueryResult::Rollback => {
                        println!("Transaction rolled back");
                    }
                }
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }
    
    Ok(())
}

fn print_help() {
    println!("SabiDB Commands:");
    println!("  CREATE TABLE name (col1 TYPE, col2 TYPE, ...)");
    println!("  INSERT INTO table (col1, col2) VALUES (val1, val2), ...");
    println!("  SELECT col1, col2 FROM table WHERE condition");
    println!("  DELETE FROM table WHERE condition");
    println!("  BEGIN TRANSACTION");
    println!("  COMMIT");
    println!("  ROLLBACK");
    println!("  exit, quit - Exit the program");
    println!("  help - Show this help");
    println!("\nExamples:");
    println!("  CREATE TABLE users (id INTEGER, name TEXT, active BOOLEAN)");
    println!("  INSERT INTO users VALUES (1, 'Alice', true)");
    println!("  SELECT * FROM users WHERE id > 0");
}