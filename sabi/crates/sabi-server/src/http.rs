//! HTTP endpoint handlers

use axum::{
    extract::{State, Path},
    Json,
    http::StatusCode,
};
use tracing::{info, warn};

use crate::{
    SabiServer,
    protocol::*,
};

/// Handle SQL query
pub async fn handle_query(
    State(server): State<std::sync::Arc<SabiServer>>,
    Json(request): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("Processing query: {}", request.sql);
    
    // Update metrics
    {
        let mut metrics = server.metrics.lock().await;
        metrics.queries_per_second += 1.0;
    }
    
    match server.execute_sql_query(&request.sql, request.args, request.at_tx).await {
        Ok(response) => Ok(Json(response)),
        Err(error) => {
            warn!("Query failed: {}", error.message);
            Err((StatusCode::BAD_REQUEST, Json(error)))
        }
    }
}

/// Handle SQL mutation
pub async fn handle_mutation(
    State(server): State<std::sync::Arc<SabiServer>>,
    Json(request): Json<MutationRequest>,
) -> Result<Json<MutationResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("Processing mutation: {}", request.sql);
    
    match server.execute_sql_mutation(&request.sql, request.args, request.client_tx).await {
        Ok(response) => Ok(Json(response)),
        Err(error) => {
            warn!("Mutation failed: {}", error.message);
            Err((StatusCode::BAD_REQUEST, Json(error)))
        }
    }
}

/// Handle deterministic function execution
pub async fn handle_function(
    State(server): State<std::sync::Arc<SabiServer>>,
    Path(name): Path<String>,
    Json(request): Json<FunctionRequest>,
) -> Result<Json<FunctionResponse>, (StatusCode, Json<ErrorResponse>)> {
    info!("Executing function: {}", name);
    
    match server.execute_function(&name, request.args).await {
        Ok(response) => Ok(Json(response)),
        Err(error) => {
            warn!("Function execution failed: {}", error.message);
            Err((StatusCode::BAD_REQUEST, Json(error)))
        }
    }
}