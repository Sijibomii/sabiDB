//! Reactive query system
//!
//! Tracks dependencies between queries and data
//! Automatically re-evaluates queries when data changes

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock, Weak};
use std::time::{Duration, Instant};
use sabi_core::TxId;
use tokio::sync::broadcast;
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::types::{JsValue, FunctionResult};
use crate::deterministic::DeterministicRuntime;
use crate::error::RuntimeError;

/// Query subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub query_name: String,
    pub args: Vec<JsValue>,
    pub last_value: Option<JsValue>,
    pub last_version: u64,
}

/// Query subscriber
#[derive(Clone)]
pub struct QuerySubscriber {
    pub id: Uuid,
    sender: broadcast::Sender<QueryUpdate>,
}

impl QuerySubscriber {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(100);
        Self {
            id: Uuid::now_v7(),
            sender,
        }
    }
    
    pub fn subscribe(&self) -> broadcast::Receiver<QueryUpdate> {
        self.sender.subscribe()
    }
    
    pub fn notify(&self, update: QueryUpdate) -> Result<(), RuntimeError> {
        self.sender.send(update)
            .map_err(|e| RuntimeError::SubscriptionError(e.to_string()))?;
        Ok(())
    }
}

/// Query update notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryUpdate {
    pub query_name: String,
    pub args: Vec<JsValue>,
    pub old_value: Option<JsValue>,
    pub new_value: JsValue,
    pub version: u64,
    pub changed_keys: Vec<String>,
}

/// Dependency graph node
#[derive(Debug, Clone)]
struct DependencyNode {
    queries: HashSet<String>,  // query_name + args hash
    subscribers: HashSet<Uuid>,
    last_version: u64,
}

/// Reactive engine
pub struct ReactiveEngine {
    /// Key → queries that depend on it
    dependencies: RwLock<HashMap<String, DependencyNode>>,
    
    /// Query → keys it reads
    query_deps: RwLock<HashMap<String, HashSet<String>>>,
    
    /// Active subscribers
    subscribers: RwLock<HashMap<Uuid, QuerySubscriber>>,
    
    /// Deterministic runtime
    runtime: Arc<DeterministicRuntime>,
    
    /// Version counter
    version: RwLock<u64>,
}

impl ReactiveEngine {
    pub fn new(runtime: Arc<DeterministicRuntime>) -> Self {
        Self {
            dependencies: RwLock::new(HashMap::new()),
            query_deps: RwLock::new(HashMap::new()),
            subscribers: RwLock::new(HashMap::new()),
            runtime,
            version: RwLock::new(0),
        }
    }
    
    /// Execute a query and track its dependencies
    pub fn execute_query(
        &self,
        tx_id: Uuid,
        query_name: &str,
        args: Vec<JsValue>,
        subscriber_id: Option<Uuid>,
    ) -> Result<FunctionResult, RuntimeError> {
        let start_time = Instant::now();
        
        // Generate query ID
        let query_id = self.query_id(query_name, &args);
        
        // Execute query
        let result = self.runtime.execute_query(TxId(tx_id), query_name, args.clone())?;
        
        // Track dependencies
        self.record_dependencies(&query_id, &result.keys_read)?;
        
        // Subscribe if requested
        if let Some(sub_id) = subscriber_id {
            self.subscribe_to_query(sub_id, query_name, args.clone(), &result)?;
        }
        
        // Update query cache
        self.update_query_cache(&query_id, &result)?;
        
        let duration = start_time.elapsed();
        print!("Query {:?} executed in {:?}", query_name, duration);
        
        Ok(result)
    }
    
    /// Execute a mutation and trigger reactive updates
    pub fn execute_mutation(
        &self,
        tx_id: Uuid,
        mutation_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let start_time = Instant::now();
        
        // Execute mutation
        let result = self.runtime.execute_mutation(TxId(tx_id), mutation_name, args)?;
        
        // Trigger reactive updates for affected keys
        self.trigger_updates(&result.keys_written)?;
        
        let duration = start_time.elapsed();
        println!("Mutation {:?} executed in {:?}", mutation_name, duration);
        
        Ok(result)
    }
    
    /// Subscribe to query updates
    pub fn subscribe(
        &self,
        query_name: &str,
        args: Vec<JsValue>,
    ) -> Result<QuerySubscriber, RuntimeError> {
        let subscriber = QuerySubscriber::new();
        let subscriber_id = subscriber.id;
        
        // Store subscriber
        let mut subscribers = self.subscribers.write()
            .map_err(|_| RuntimeError::LockPoisoned("subscribers".into()))?;
        subscribers.insert(subscriber_id, subscriber.clone());
        
        // Execute query to establish dependencies
        let dummy_tx = Uuid::now_v7();
        self.execute_query(dummy_tx, query_name, args.clone(), Some(subscriber_id))?;
        
        Ok(subscriber)
    }
    
    /// Unsubscribe from query updates
    pub fn unsubscribe(&self, subscriber_id: Uuid) -> Result<(), RuntimeError> {
        let mut subscribers = self.subscribers.write()
            .map_err(|_| RuntimeError::LockPoisoned("subscribers".into()))?;
        subscribers.remove(&subscriber_id);
        
        // Remove from dependency graph
        let mut deps = self.dependencies.write()
            .map_err(|_| RuntimeError::LockPoisoned("dependencies".into()))?;
        
        for node in deps.values_mut() {
            node.subscribers.remove(&subscriber_id);
        }
        
        Ok(())
    }
    
    /// Record query dependencies
    fn record_dependencies(
        &self,
        query_id: &str,
        keys_read: &[String],
    ) -> Result<(), RuntimeError> {
        let mut query_deps = self.query_deps.write()
            .map_err(|_| RuntimeError::LockPoisoned("query_deps".into()))?;
        
        // Store what keys this query reads
        let deps_set: HashSet<String> = keys_read.iter().cloned().collect();
        query_deps.insert(query_id.to_string(), deps_set.clone());
        
        // Update reverse mapping (key → queries)
        let mut dependencies = self.dependencies.write()
            .map_err(|_| RuntimeError::LockPoisoned("dependencies".into()))?;
        
        for key in deps_set {
            let node = dependencies.entry(key).or_insert_with(|| DependencyNode {
                queries: HashSet::new(),
                subscribers: HashSet::new(),
                last_version: 0,
            });
            node.queries.insert(query_id.to_string());
        }
        
        Ok(())
    }
    
    /// Subscribe to query
    fn subscribe_to_query(
        &self,
        subscriber_id: Uuid,
        query_name: &str,
        args: Vec<JsValue>,
        result: &FunctionResult,
    ) -> Result<(), RuntimeError> {
        let query_id = self.query_id(query_name, &args);
        
        let mut dependencies = self.dependencies.write()
            .map_err(|_| RuntimeError::LockPoisoned("dependencies".into()))?;
        
        // Add subscriber to all keys this query reads
        for key in &result.keys_read {
            if let Some(node) = dependencies.get_mut(key) {
                node.subscribers.insert(subscriber_id);
            }
        }
        
        Ok(())
    }
    
    /// Update query cache
    fn update_query_cache(
        &self,
        query_id: &str,
        result: &FunctionResult,
    ) -> Result<(), RuntimeError> {
        // Increment version
        let mut version = self.version.write()
            .map_err(|_| RuntimeError::LockPoisoned("version".into()))?;
        *version += 1;
        let current_version = *version;
        
        // Update last version for all read keys
        let mut dependencies = self.dependencies.write()
            .map_err(|_| RuntimeError::LockPoisoned("dependencies".into()))?;
        
        for key in &result.keys_read {
            if let Some(node) = dependencies.get_mut(key) {
                node.last_version = current_version;
            }
        }
        
        Ok(())
    }
    
    /// Trigger updates when keys change
    fn trigger_updates(&self, changed_keys: &[String]) -> Result<(), RuntimeError> {
        let mut queries_to_recompute = HashSet::new();
        let mut subscribers_to_notify = HashMap::new();
        
        // Find affected queries
        let dependencies = self.dependencies.read()
            .map_err(|_| RuntimeError::LockPoisoned("dependencies".into()))?;
        
        for key in changed_keys {
            if let Some(node) = dependencies.get(key) {
                queries_to_recompute.extend(node.queries.iter().cloned());
                for sub_id in &node.subscribers {
                    subscribers_to_notify.entry(*sub_id)
                        .or_insert_with(HashSet::new)
                        .insert(key.clone());
                }
            }
        }
        
        // Recompute affected queries
        for query_id in queries_to_recompute {
            self.recompute_query(&query_id)?;
        }
        
        // Notify subscribers
        let subscribers = self.subscribers.read()
            .map_err(|_| RuntimeError::LockPoisoned("subscribers".into()))?;
        
        for (sub_id, changed_keys_set) in subscribers_to_notify {
            if let Some(subscriber) = subscribers.get(&sub_id) {
                // TODO: Send notification with query result
                let update = QueryUpdate {
                    query_name: "todo".to_string(),
                    args: vec![],
                    old_value: None,
                    new_value: JsValue::Null,
                    version: 0,
                    changed_keys: changed_keys_set.into_iter().collect(),
                };
                
                let _ = subscriber.notify(update);
            }
        }
        
        Ok(())
    }
    
    /// Recompute a query
    fn recompute_query(&self, query_id: &str) -> Result<(), RuntimeError> {
        // Parse query_id to get name and args
        // This is simplified - you'd need to store the actual query definitions
        print!("Recomputing query: {:?}", query_id);
        Ok(())
    }
    
    /// Generate unique query ID
    fn query_id(&self, query_name: &str, args: &[JsValue]) -> String {
        use sha2::{Sha256, Digest};
        
        let mut hasher = Sha256::new();
        hasher.update(query_name.as_bytes());
        
        for arg in args {
            let arg_json = serde_json::to_string(arg).unwrap_or_default();
            hasher.update(arg_json.as_bytes());
        }
        
        let hash = hasher.finalize();
        format!("{}:{:x}", query_name, hash)
    }
}