//! Track connected clients and their subscriptions

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;
use serde_json::Value;

/// Client state with subscriptions
#[derive(Debug, Clone)]
pub struct ClientState {
    pub client_id: Uuid,
    pub last_seen_tx: Option<u64>,
    pub connected_at: Instant,
    pub last_heartbeat: Instant,
    pub subscriptions: HashMap<String, SubscriptionInfo>,
}

/// Subscription information
#[derive(Debug, Clone)]
pub struct SubscriptionInfo {
    pub query: String,
    pub args: Vec<Value>,
    pub created_at: Instant,
    pub last_update_tx: Option<u64>,
}

/// Track all connected clients
pub struct ClientTracker {
    clients: HashMap<Uuid, ClientState>,
    subscription_to_clients: HashMap<String, HashSet<Uuid>>,
}

impl ClientTracker {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            subscription_to_clients: HashMap::new(),
        }
    }
    
    /// Add new client
    pub fn add_client(&mut self, client_id: Uuid) {
        let state = ClientState {
            client_id,
            last_seen_tx: None,
            connected_at: Instant::now(),
            last_heartbeat: Instant::now(),
            subscriptions: HashMap::new(),
        };
        
        self.clients.insert(client_id, state);
    }
    
    /// Remove client and all their subscriptions
    pub fn remove_client(&mut self, client_id: Uuid) {
        if let Some(state) = self.clients.remove(&client_id) {
            // Remove from subscription mapping
            for subscription_id in state.subscriptions.keys() {
                if let Some(clients) = self.subscription_to_clients.get_mut(subscription_id) {
                    clients.remove(&client_id);
                    if clients.is_empty() {
                        self.subscription_to_clients.remove(subscription_id);
                    }
                }
            }
        }
    }
    
    /// Update client's last seen transaction
    pub fn update_last_seen_tx(&mut self, client_id: Uuid, last_seen_tx: Option<u64>) {
        if let Some(state) = self.clients.get_mut(&client_id) {
            state.last_seen_tx = last_seen_tx;
            state.last_heartbeat = Instant::now();
        }
    }
    
    /// Add subscription for client
    pub fn add_subscription(
        &mut self,
        client_id: Uuid,
        subscription_id: &str,
        query: &str,
        args: Vec<Value>,
    ) {
        if let Some(state) = self.clients.get_mut(&client_id) {
            let info = SubscriptionInfo {
                query: query.to_string(),
                args,
                created_at: Instant::now(),
                last_update_tx: None,
            };
            
            state.subscriptions.insert(subscription_id.to_string(), info);
            state.last_heartbeat = Instant::now();
        }
        
        // Update reverse mapping
        self.subscription_to_clients
            .entry(subscription_id.to_string())
            .or_default()
            .insert(client_id);
    }
    
    /// Remove subscription for client
    pub fn remove_subscription(&mut self, client_id: Uuid, subscription_id: &str) {
        // Remove from client state
        if let Some(state) = self.clients.get_mut(&client_id) {
            state.subscriptions.remove(subscription_id);
            state.last_heartbeat = Instant::now();
        }
        
        // Remove from reverse mapping
        if let Some(clients) = self.subscription_to_clients.get_mut(subscription_id) {
            clients.remove(&client_id);
            if clients.is_empty() {
                self.subscription_to_clients.remove(subscription_id);
            }
        }
    }
    
    /// Get all clients subscribed to a particular key/query
    pub fn get_clients_for_key(&self, key: &str) -> Vec<Uuid> {
        // This is simplified - in reality you'd need to map keys to subscriptions
        // For now, return all clients (they'll filter on their end)
        self.clients.keys().cloned().collect()
    }
    
    /// Get client state
    pub fn get_client(&self, client_id: Uuid) -> Option<&ClientState> {
        self.clients.get(&client_id)
    }
    
    /// Get all clients
    pub fn get_all_clients(&self) -> Vec<ClientState> {
        self.clients.values().cloned().collect()
    }
    
    /// Clean up stale clients (haven't sent heartbeat in timeout)
    pub fn cleanup_stale_clients(&mut self, timeout: Duration) -> Vec<Uuid> {
        let now = Instant::now();
        let stale_clients: Vec<Uuid> = self.clients
            .iter()
            .filter(|(_, state)| now.duration_since(state.last_heartbeat) > timeout)
            .map(|(id, _)| *id)
            .collect();
        
        for client_id in &stale_clients {
            self.remove_client(*client_id);
        }
        
        stale_clients
    }
    
    /// Get number of connected clients
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
    
    /// Get number of active subscriptions
    pub fn subscription_count(&self) -> usize {
        self.subscription_to_clients.len()
    }
}