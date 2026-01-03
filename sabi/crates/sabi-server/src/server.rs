//! Main server implementation

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Mutex, mpsc};
use tokio::time::interval;
use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    response::Response,
    routing::{post, get},
    Json,
};
use tower_http::trace::TraceLayer;
use tracing::{info, warn, error};
use uuid::Uuid;

use sabi_core::TxId;
use sabi_storage::{StorageEngine, Transaction};
use sabi_sql::{QueryParser, QueryPlanner, QueryExecutor as SqlExecutor, Catalog};
use sabi_runtime::{DeterministicRuntime, ReactiveEngine, QueryExecutor, MutationExecutor};

use crate::{
    http::{handle_query, handle_mutation, handle_function},
    websocket::WebSocketHandler,
    protocol::*,
    client_tracker::ClientTracker,
};

/// Main server state
pub struct SabiServer {
    storage: Arc<StorageEngine>,
    sql_parser: QueryParser,
    sql_planner: QueryPlanner,
    sql_executor: SqlExecutor,
    deterministic_runtime: Arc<DeterministicRuntime>,
    reactive_engine: Arc<ReactiveEngine>,
    query_executor: QueryExecutor,
    mutation_executor: MutationExecutor,
    client_tracker: Arc<RwLock<ClientTracker>>,
    current_tx_id: Arc<RwLock<u64>>,
    metrics: Arc<Mutex<ServerMetrics>>,
    shutdown_tx: mpsc::Sender<()>,
}

impl SabiServer {
    /// Create a new server instance
    pub async fn new(
        storage: Arc<StorageEngine>,
        catalog: Catalog,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Initialize SQL components
        let sql_parser = QueryParser::new();
        let sql_planner = QueryPlanner::new(catalog.clone());
        let sql_executor = SqlExecutor::new(storage.clone(), catalog);
        
        // Initialize deterministic runtime
        let deterministic_runtime = Arc::new(DeterministicRuntime::new()?);
        let reactive_engine = Arc::new(ReactiveEngine::new(deterministic_runtime.clone()));
        
        // Initialize executors
        let query_executor = QueryExecutor::new(
            reactive_engine.clone(),
            storage.clone(),
            100, // max concurrent queries
        );
        
        let storage_mutex = Arc::new(Mutex::new(storage.as_ref().clone()));
        let mutation_executor = MutationExecutor::new(
            deterministic_runtime.clone(),
            reactive_engine.clone(),
            storage_mutex,
        );
        
        // Initialize client tracker
        let client_tracker = Arc::new(RwLock::new(ClientTracker::new()));
        
        // Get current transaction ID
        let current_tx = {
            let storage = storage.lock().map_err(|_| "Storage lock poisoned")?;
            storage.get_latest_tx_id()? as u64
        };
        
        let current_tx_id = Arc::new(RwLock::new(current_tx));
        
        // Initialize metrics
        let metrics = Arc::new(Mutex::new(ServerMetrics {
            connections: 0,
            subscriptions: 0,
            queries_per_second: 0.0,
            mutations_per_second: 0.0,
            current_tx_id: current_tx,
            uptime_seconds: 0,
        }));
        
        // Setup shutdown channel
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        
        // Start background tasks
        let metrics_clone = metrics.clone();
        let current_tx_id_clone = current_tx_id.clone();
        let storage_clone = storage.clone();
        
        tokio::spawn(async move {
            Self::background_tasks(metrics_clone, current_tx_id_clone, storage_clone, shutdown_rx).await;
        });
        
        Ok(Self {
            storage,
            sql_parser,
            sql_planner,
            sql_executor,
            deterministic_runtime,
            reactive_engine,
            query_executor,
            mutation_executor,
            client_tracker,
            current_tx_id,
            metrics,
            shutdown_tx,
        })
    }
    
    /// Start the server
    pub async fn start(self: Arc<Self>, addr: &str) -> Result<(), Box<dyn std::error::Error>> {
        let app = self.create_router();
        
        info!("Starting SabiDB server on {}", addr);
        
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(self.shutdown_signal())
            .await?;
        
        Ok(())
    }
    
    /// Create Axum router
    fn create_router(self: Arc<Self>) -> Router {
        let state = Arc::clone(&self);
        
        Router::new()
            // HTTP endpoints
            .route("/query", post(handle_query))
            .route("/mutate", post(handle_mutation))
            .route("/fn/:name", post(handle_function))
            .route("/health", get(|| async { "OK" }))
            .route("/metrics", get(Self::handle_metrics))
            
            // WebSocket endpoint
            .route("/ws", get(move |ws: WebSocketUpgrade| {
                let handler = WebSocketHandler::new(Arc::clone(&state));
                async move {
                    ws.on_upgrade(move |socket| handler.handle_connection(socket))
                }
            }))
            
            // State and middleware
            .with_state(state)
            .layer(TraceLayer::new_for_http())
    }
    
    /// Handle metrics endpoint
    async fn handle_metrics(
        State(server): State<Arc<Self>>,
    ) -> Json<ServerMetrics> {
        let metrics = server.metrics.lock().await;
        Json(metrics.clone())
    }
    
    /// Background tasks for metrics, cleanup, etc.
    async fn background_tasks(
        metrics: Arc<Mutex<ServerMetrics>>,
        current_tx_id: Arc<RwLock<u64>>,
        storage: Arc<StorageEngine>,
        mut shutdown_rx: mpsc::Receiver<()>,
    ) {
        let mut interval = interval(Duration::from_secs(1));
        let start_time = Instant::now();
        
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Update metrics
                    let mut metrics_guard = metrics.lock().await;
                    metrics_guard.uptime_seconds = start_time.elapsed().as_secs();
                    
                    // Update current transaction ID
                    if let Ok(storage_guard) = storage.lock() {
                        if let Ok(tx_id) = storage_guard.get_latest_tx_id() {
                            *current_tx_id.write().await = tx_id as u64;
                            metrics_guard.current_tx_id = tx_id as u64;
                        }
                    }
                    
                    // Reset per-second counters (simplified)
                    metrics_guard.queries_per_second = 0.0;
                    metrics_guard.mutations_per_second = 0.0;
                }
                _ = shutdown_rx.recv() => {
                    info!("Shutting down background tasks");
                    break;
                }
            }
        }
    }
    
    /// Shutdown signal handler
    async fn shutdown_signal(&self) {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Shutdown signal received");
        
        // Clean shutdown
        let _ = self.shutdown_tx.send(()).await;
    }
    
    /// Execute SQL query
    pub async fn execute_sql_query(
        &self,
        sql: &str,
        args: Vec<serde_json::Value>,
        at_tx: Option<u64>,
    ) -> Result<QueryResponse, ErrorResponse> {
        // Parse SQL
        let statements = self.sql_parser.parse(sql)
            .map_err(|e| ErrorResponse {
                code: ErrorCode::SyntaxError,
                message: format!("SQL parsing error: {}", e),
                details: None,
            })?;
        
        // For now, only handle single SELECT statements
        let stmt = statements.first()
            .ok_or_else(|| ErrorResponse {
                code: ErrorCode::SyntaxError,
                message: "No SQL statement provided".into(),
                details: None,
            })?;
        
        // Plan query
        let plan = self.sql_planner.plan(stmt.clone())
            .map_err(|e| ErrorResponse {
                code: ErrorCode::SyntaxError,
                message: format!("Query planning error: {}", e),
                details: None,
            })?;
        
        // Execute query
        let result = self.sql_executor.execute(plan)
            .map_err(|e| ErrorResponse {
                code: ErrorCode::Internal,
                message: format!("Query execution error: {}", e),
                details: None,
            })?;
        
        // Convert to response
        match result {
            sabi_sql::QueryResult::Select { columns, rows } => {
                let json_rows: Vec<serde_json::Value> = rows.into_iter()
                    .map(|row| {
                        let mut obj = serde_json::Map::new();
                        for (i, col) in columns.iter().enumerate() {
                            if i < row.len() {
                                let value = match &row[i] {
                                    sabi_sql::Value::Null => serde_json::Value::Null,
                                    sabi_sql::Value::Integer(n) => serde_json::Value::Number((*n).into()),
                                    sabi_sql::Value::Text(s) => serde_json::Value::String(s.clone()),
                                    sabi_sql::Value::Boolean(b) => serde_json::Value::Bool(*b),
                                };
                                obj.insert(col.clone(), value);
                            }
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect();
                
                let read_tx = at_tx.unwrap_or_else(|| {
                    // Use latest transaction ID
                    tokio::task::block_in_place(|| {
                        let storage = self.storage.lock().unwrap();
                        storage.get_latest_tx_id().unwrap_or(0) as u64
                    })
                });
                
                Ok(QueryResponse {
                    rows: json_rows,
                    read_tx,
                })
            }
            _ => Err(ErrorResponse {
                code: ErrorCode::SyntaxError,
                message: "Only SELECT queries are supported via HTTP".into(),
                details: None,
            }),
        }
    }
    
    /// Execute SQL mutation
    pub async fn execute_sql_mutation(
        &self,
        sql: &str,
        args: Vec<serde_json::Value>,
        client_tx: u64,
    ) -> Result<MutationResponse, ErrorResponse> {
        // TODO: Implement actual mutation execution
        // For now, simulate a successful mutation
        
        // Get next transaction ID
        let tx_id = {
            let mut current = self.current_tx_id.write().await;
            *current += 1;
            *current
        };
        
        // Update metrics
        {
            let mut metrics = self.metrics.lock().await;
            metrics.mutations_per_second += 1.0;
        }
        
        Ok(MutationResponse {
            tx_id,
            affected_rows: 1, // Simulated
        })
    }
    
    /// Execute deterministic function
    pub async fn execute_function(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<FunctionResponse, ErrorResponse> {
        // Convert args to JsValue
        use sabi_runtime::JsValue;
        let js_args = match args {
            serde_json::Value::Array(arr) => {
                arr.into_iter()
                    .map(|v| serde_json::from_value(v).map_err(|e| ErrorResponse {
                        code: ErrorCode::SyntaxError,
                        message: format!("Invalid argument: {}", e),
                        details: None,
                    }))
                    .collect::<Result<Vec<JsValue>, _>>()?
            }
            _ => return Err(ErrorResponse {
                code: ErrorCode::SyntaxError,
                message: "Function arguments must be an array".into(),
                details: None,
            }),
        };
        
        // Execute mutation
        let mutation_def = sabi_runtime::MutationDef {
            name: name.to_string(),
            function: "".to_string(), // Already registered
            args: js_args,
        };
        
        let result = self.mutation_executor.execute_mutation(mutation_def).await
            .map_err(|e| ErrorResponse {
                code: ErrorCode::Internal,
                message: format!("Function execution error: {}", e),
                details: None,
            })?;
        
        // Get transaction ID (simulated)
        let tx_id = {
            let mut current = self.current_tx_id.write().await;
            *current += 1;
            *current
        };
        
        // Convert result to JSON
        let result_json = serde_json::to_value(result.value)
            .map_err(|e| ErrorResponse {
                code: ErrorCode::Internal,
                message: format!("Result serialization error: {}", e),
                details: None,
            })?;
        
        Ok(FunctionResponse {
            tx_id,
            result: result_json,
        })
    }
    
    /// Create subscription
    pub async fn create_subscription(
        &self,
        client_id: Uuid,
        query: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<String, ErrorResponse> {
        let subscription_id = format!("sub-{}", Uuid::new_v4());
        
        // Store subscription in client tracker
        {
            let mut tracker = self.client_tracker.write().await;
            tracker.add_subscription(client_id, &subscription_id, query, args);
        }
        
        // Update metrics
        {
            let mut metrics = self.metrics.lock().await;
            metrics.subscriptions += 1;
        }
        
        info!("Created subscription {} for client {}", subscription_id, client_id);
        
        Ok(subscription_id)
    }
    
    /// Remove subscription
    pub async fn remove_subscription(
        &self,
        client_id: Uuid,
        subscription_id: &str,
    ) -> Result<(), ErrorResponse> {
        {
            let mut tracker = self.client_tracker.write().await;
            tracker.remove_subscription(client_id, subscription_id);
        }
        
        // Update metrics
        {
            let mut metrics = self.metrics.lock().await;
            metrics.subscriptions = metrics.subscriptions.saturating_sub(1);
        }
        
        info!("Removed subscription {} for client {}", subscription_id, client_id);
        
        Ok(())
    }
    
    /// Send updates to client
    pub async fn send_update(
        &self,
        client_id: Uuid,
        update: UpdateMessage,
    ) -> Result<(), ErrorResponse> {
        // This would normally send via WebSocket
        // For now, just log
        info!("Would send update to client {}: {:?}", client_id, update);
        Ok(())
    }
    
    /// Get server metrics
    pub async fn get_metrics(&self) -> ServerMetrics {
        self.metrics.lock().await.clone()
    }
}

// StorageEngine extension trait
trait StorageEngineExt {
    fn get_latest_tx_id(&self) -> Result<u64, String>;
}

impl StorageEngineExt for StorageEngine {
    fn get_latest_tx_id(&self) -> Result<u64, String> {
        // This is a simplified implementation
        // In reality, you'd get this from the transaction manager
        Ok(1000) // Example value
    }
}