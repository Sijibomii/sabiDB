//! Execution orchestration layer

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;
use tokio::sync::{Mutex, Semaphore};
use futures::future::BoxFuture;

use sabi_core::TxId;
use sabi_storage::engine::{StorageEngine};

use crate::deterministic::DeterministicRuntime;
use crate::reactive::{ReactiveEngine, QuerySubscriber};
use crate::types::{JsValue, FunctionResult, QueryDef, MutationDef, ActionDef};
use crate::error::RuntimeError;

/// Query executor with reactive capabilities
pub struct QueryExecutor {
    reactive: Arc<ReactiveEngine>,
    storage: Arc<StorageEngine>,
    max_concurrent: Semaphore,
}

impl QueryExecutor {
    pub fn new(
        reactive: Arc<ReactiveEngine>,
        storage: Arc<StorageEngine>,
        max_concurrent: usize,
    ) -> Self {
        Self {
            reactive,
            storage,
            max_concurrent: Semaphore::new(max_concurrent),
        }
    }
    
    /// Execute a query
    pub async fn execute_query(
        &self,
        query_def: QueryDef,
        subscribe: bool,
    ) -> Result<(FunctionResult, Option<QuerySubscriber>), RuntimeError> {
        let _permit = self.max_concurrent.acquire().await
            .map_err(|e| RuntimeError::FunctionError(e.to_string()))?;
        
        // Begin transaction
        let tx = {
            self.storage.begin(true) // Read-only
        };
        
        // Execute query
        let subscriber_id = if subscribe {
            Some(Uuid::now_v7())
        } else {
            None
        };
        
        let result = self.reactive.execute_query(
            tx.tx_id.0,
            &query_def.name,
            query_def.args.clone(),
            subscriber_id,
        )?;
        
        let subscriber = if subscribe {
            Some(self.reactive.subscribe(&query_def.name, query_def.args)?)
        } else {
            None
        };
        
        Ok((result, subscriber))
    }
    
    /// Get query result without reactive subscription
    pub async fn query_once(&self, query_def: QueryDef) -> Result<FunctionResult, RuntimeError> {
        let (result, _) = self.execute_query(query_def, false).await?;
        Ok(result)
    }
}

/// Mutation executor with WAL logging
pub struct MutationExecutor {
    deterministic: Arc<DeterministicRuntime>,
    reactive: Arc<ReactiveEngine>,
    storage: Arc<Mutex<StorageEngine>>,
    wal_position: RwLock<u64>,
}

impl MutationExecutor {
    pub fn new(
        deterministic: Arc<DeterministicRuntime>,
        reactive: Arc<ReactiveEngine>,
        storage: Arc<Mutex<StorageEngine>>,
    ) -> Self {
        Self {
            deterministic,
            reactive,
            storage,
            wal_position: RwLock::new(0),
        }
    }
    
    /// Execute a mutation
    pub async fn execute_mutation(
        &self,
        mutation_def: MutationDef,
    ) -> Result<FunctionResult, RuntimeError> {
        // Generate transaction ID
        let tx_id = Uuid::now_v7();
        
        // Log mutation start to WAL
        self.log_mutation_start(&mutation_def, tx_id)?;
        
        // Execute mutation
        let result = self.reactive.execute_mutation(
            tx_id,
            &mutation_def.name,
            mutation_def.clone().args,
        )?;
        
        // Log mutation result to WAL
        self.log_mutation_result(&mutation_def, tx_id, &result)?;
        
        // Commit transaction
        {
            let storage = self.storage.lock().await;
            storage.commit(TxId(tx_id))?;
        }
        
        // Update WAL position
        self.advance_wal_position()?;
        
        Ok(result)
    }
    
    fn log_mutation_start(
        &self,
        mutation_def: &MutationDef,
        tx_id: Uuid,
    ) -> Result<(), RuntimeError> {
        let wal_pos = *self.wal_position.read()
            .map_err(|_| RuntimeError::LockPoisoned("wal_position".into()))?;
        
        // In production, you would write to actual WAL
        println!("INFO: WAL[{}]: Mutation {} started, tx={}",wal_pos,mutation_def.name,tx_id);
        
        Ok(())
    }
    
    fn log_mutation_result(
        &self,
        mutation_def: &MutationDef,
        tx_id: Uuid,
        result: &FunctionResult,
    ) -> Result<(), RuntimeError> {
        let wal_pos = *self.wal_position.read()
            .map_err(|_| RuntimeError::LockPoisoned("wal_position".into()))?;
        
        // Log deterministic result
        println!(
            "WAL[{}]: Mutation {} completed, tx={}, writes={:?}",
            wal_pos,
            mutation_def.name,
            tx_id,
            result.keys_written
        );
        
        Ok(())
    }
    
    fn advance_wal_position(&self) -> Result<(), RuntimeError> {
        let mut pos = self.wal_position.write()
            .map_err(|_| RuntimeError::LockPoisoned("wal_position".into()))?;
        *pos += 1;
        Ok(())
    }
}

/// Action executor for external effects
pub struct ActionExecutor {
    deterministic: Arc<DeterministicRuntime>,
    storage: Arc<Mutex<StorageEngine>>,
    external_services: HashMap<String, ExternalService>,
}

impl ActionExecutor {
    pub fn new(
        deterministic: Arc<DeterministicRuntime>,
        storage: Arc<Mutex<StorageEngine>>,
    ) -> Self {
        Self {
            deterministic,
            storage,
            external_services: HashMap::new(),
        }
    }
    
    /// Register external service
    pub fn register_service(&mut self, name: &str, service: ExternalService) {
        self.external_services.insert(name.to_string(), service);
    }
    
    /// Execute an action (mutation with external effects)
    pub async fn execute_action(
        &self,
        action_def: ActionDef,
    ) -> Result<FunctionResult, RuntimeError> {
        // Actions are like mutations but can have external effects
        // They still need to be deterministic for replay
        
        let tx_id = Uuid::now_v7();
        
        // Log action start
        self.log_action_start(&action_def, tx_id)?;
        
        // Execute in deterministic runtime
        let result = self.deterministic.execute_action(
            TxId(tx_id),
            &action_def.name,
            action_def.args.clone(),
        )?;
        
        // Perform external effects (logged to WAL)
        self.execute_external_effects(&action_def, &result).await?;
        
        // Log action completion
        self.log_action_completion(&action_def, tx_id, &result)?;
        
        // Commit internal changes
        {
            let storage = self.storage.lock().await;
            storage.commit(TxId(tx_id))?;
        }
        
        Ok(result)
    }
    
    async fn execute_external_effects(
        &self,
        action_def: &ActionDef,
        result: &FunctionResult,
    ) -> Result<(), RuntimeError> {
        // External effects must be idempotent and logged
        // For example: sending emails, calling webhooks
        
        print!(
            "Action {} external effects: {:?}",
            action_def.name,
            result.keys_written
        );
        
        // Here you would implement actual external service calls
        // Each call should be logged to WAL with enough info to replay
        
        Ok(())
    }
    
    fn log_action_start(
        &self,
        action_def: &ActionDef,
        tx_id: Uuid,
    ) -> Result<(), RuntimeError> {
        print!(
            "Action {} started, tx={}, args={:?}",
            action_def.name,
            tx_id,
            action_def.args
        );
        Ok(())
    }
    
    fn log_action_completion(
        &self,
        action_def: &ActionDef,
        tx_id: Uuid,
        result: &FunctionResult,
    ) -> Result<(), RuntimeError> {
        print!(
            "Action {} completed, tx={}, result={:?}",
            action_def.name,
            tx_id,
            result.value
        );
        Ok(())
    }
}

/// External service for actions
pub struct ExternalService {
    pub name: String,
    pub execute: Box<dyn Fn(JsValue) -> BoxFuture<'static, Result<JsValue, RuntimeError>> + Send + Sync>,
}

impl ExternalService {
    pub fn new<F, Fut>(name: &str, executor: F) -> Self
    where
        F: Fn(JsValue) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<JsValue, RuntimeError>> + Send + 'static,
    {
        Self {
            name: name.to_string(),
            execute: Box::new(move |args| Box::pin(executor(args))),
        }
    }
}