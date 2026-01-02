//! Deterministic execution environment
//!
//! Rules:
//! - No system randomness
//! - No wall-clock time
//! - No external I/O
//! - All operations must be deterministic given same inputs

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use serde_json::Value as JsonValue;
use deno_core::{JsRuntime, RuntimeOptions, serde_v8, v8};
use sabi_core::TxId;

use crate::types::{JsValue, DeterministicEnv, FunctionResult, TransactionContext};
use crate::error::RuntimeError;

/// Deterministic JavaScript runtime
pub struct DeterministicRuntime {
    /// JavaScript runtime instances (one per isolate)
    runtime: Arc<Mutex<JsRuntime>>,
    
    /// Environment for deterministic operations
    env: Arc<Mutex<HashMap<TxId, DeterministicEnv>>>,
    
    /// Global function registry
    functions: Arc<Mutex<HashMap<String, String>>>,
}

impl DeterministicRuntime {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut runtime = JsRuntime::new(RuntimeOptions {
            will_snapshot: true,
            extensions: vec![
                // Register deterministic APIs
                deno_core::Extension::builder()
                    .js(deno_core::include_js_files!(
                        "deterministic_apis",
                        "src/deterministic/apis.js",
                    ))
                    .build(),
            ],
            ..Default::default()
        });
        
        // Initialize deterministic APIs
        Self::init_deterministic_apis(&mut runtime)?;
        
        Ok(Self {
            runtime: Arc::new(Mutex::new(runtime)),
            env: Arc::new(Mutex::new(HashMap::new())),
            functions: Arc::new(Mutex::new(HashMap::new())),
        })
    }
    
    fn init_deterministic_apis(runtime: &mut JsRuntime) -> Result<(), RuntimeError> {
        // Add deterministic Math.random replacement
        let code = r#"
        // Deterministic random number generator
        let _randomSeed = 0;
        
        // Replace Math.random with deterministic version
        const _originalMathRandom = Math.random;
        Math.random = function() {
            // Xorshift algorithm
            _randomSeed ^= _randomSeed << 13;
            _randomSeed ^= _randomSeed >> 7;
            _randomSeed ^= _randomSeed << 17;
            return (_randomSeed >>> 0) / 4294967296;
        };
        
        // Replace Date.now with deterministic version
        const _originalDateNow = Date.now;
        Date.now = function() {
            return globalThis.__deterministicTime || 0;
        };
        
        // Replace new Date() with deterministic version
        const _originalDate = Date;
        Date = function(...args) {
            if (args.length === 0) {
                return new _originalDate(globalThis.__deterministicTime || 0);
            }
            return new _originalDate(...args);
        };
        
        // Prohibit certain non-deterministic APIs
        Object.freeze(Math.random);
        Object.freeze(Date.now);
        Object.freeze(Date);
        
        // Provide deterministic query and mutation context
        globalThis.db = {
            query: (q, ...args) => {
                return globalThis.__executeQuery(q, args);
            },
            mutation: (m, ...args) => {
                return globalThis.__executeMutation(m, args);
            },
            // Read operations - track dependencies
            get: (key) => {
                globalThis.__recordRead(key);
                return globalThis.__storageGet(key);
            },
            // Write operations - go through WAL
            put: (key, value) => {
                globalThis.__recordWrite(key);
                globalThis.__storagePut(key, value);
            },
            delete: (key) => {
                globalThis.__recordWrite(key);
                globalThis.__storageDelete(key);
            },
        };
        
        console.log("Deterministic runtime initialized");
        "#;
        
        runtime.execute_script("[deterministic]", code.into())
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to init APIs: {}", e)))?;
        
        Ok(())
    }
    
    /// Register a function for later execution
    pub fn register_function(&self, name: &str, source: &str) -> Result<(), RuntimeError> {
        let mut functions = self.functions.lock()?;
        functions.insert(name.to_string(), source.to_string());
        Ok(())
    }
    
    /// Execute a query function
    pub fn execute_query(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, true)?;
        self.execute_function("query", function_name, args, env)
    }
    
    /// Execute a mutation function
    pub fn execute_mutation(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, false)?;
        self.execute_function("mutation", function_name, args, env)
    }
    
    /// Execute an action function (read-write with external effects)
    pub fn execute_action(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, false)?;
        self.execute_function("action", function_name, args, env)
    }
    
    fn create_env(&self, tx_id: TxId, is_read_only: bool) -> Result<DeterministicEnv, RuntimeError> {
        let mut env_map = self.env.lock()?;
        
        if !env_map.contains_key(&tx_id) {
            // Create new deterministic environment
            let seed = Self::generate_deterministic_seed(tx_id);
            let context = TransactionContext {
                tx_id: tx_id.0,
                start_time: 0,  // Will be set by WAL position
                is_read_only,
            };
            
            env_map.insert(tx_id, DeterministicEnv {
                random_seed: seed,
                logical_time: 0,
                context,
            });
        }
        
        Ok(env_map[&tx_id].clone())
    }
    
    fn execute_function(
        &self,
        function_type: &str,
        function_name: &str,
        args: Vec<JsValue>,
        env: DeterministicEnv,
    ) -> Result<FunctionResult, RuntimeError> {
        let start_time = std::time::Instant::now();
        
        // Get function source
        let functions = self.functions.lock()?;
        let source = functions.get(function_name)
            .ok_or_else(|| RuntimeError::InvalidFunction(function_name.to_string()))?;
        
        // Execute in JavaScript runtime
        let mut runtime = self.runtime.lock()?;
        
        // Set up execution environment
        self.prepare_runtime(&mut runtime, env, &args)?;
        
        // Execute the function
        let js_args: Vec<JsonValue> = args.iter()
            .map(|v| serde_json::to_value(v).unwrap())
            .collect();
        
        let js_args_str = serde_json::to_string(&js_args)
            .map_err(RuntimeError::SerializationError)?;
        
        let js_code = format!(
            r#"
            try {{
                globalThis.__readKeys = new Set();
                globalThis.__writtenKeys = new Set();
                
                // Execute the function
                const result = ({})(...{});
                
                // Collect dependency info
                const readKeys = Array.from(globalThis.__readKeys);
                const writtenKeys = Array.from(globalThis.__writtenKeys);
                
                {{ 
                    value: result,
                    readKeys,
                    writtenKeys
                }}
            }} catch (error) {{
                {{ error: error.message }}
            }}
            "#,
            source, js_args_str
        );
        
        let result = runtime.execute_script(&format!("[{}]", function_name), js_code.into())
            .map_err(|e| RuntimeError::FunctionError(format!("Execution failed: {}", e)))?;
        
        let result_value = runtime.poll_event_loop(false, None)
            .map_err(|e| RuntimeError::FunctionError(format!("Event loop failed: {}", e)))?;
        
        // Extract result from V8
        let scope = &mut runtime.handle_scope();
        let local_result = v8::Local::new(scope, result);
        let result_json: JsonValue = serde_v8::from_v8(scope, local_result)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to deserialize result: {}", e)))?;
        
        // Parse result
        if let Some(error) = result_json.get("error") {
            return Err(RuntimeError::FunctionError(error.as_str().unwrap().to_string()));
        }
        
        let value = serde_json::from_value(result_json.get("value").unwrap().clone())
            .map_err(RuntimeError::SerializationError)?;
        
        let read_keys: Vec<String> = result_json.get("readKeys")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        
        let written_keys: Vec<String> = result_json.get("writtenKeys")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        
        let duration_ns = start_time.elapsed().as_nanos() as u64;
        
        Ok(FunctionResult {
            value,
            keys_read: read_keys,
            keys_written: written_keys,
            duration_ns,
        })
    }
    
    fn prepare_runtime(
        &self,
        runtime: &mut JsRuntime,
        env: DeterministicEnv,
        args: &[JsValue],
    ) -> Result<(), RuntimeError> {
        // Set deterministic random seed
        let seed_code = format!("_randomSeed = {};", env.random_seed);
        runtime.execute_script("[set_seed]", seed_code.into())
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to set seed: {}", e)))?;
        
        // Set deterministic time
        let time_code = format!("globalThis.__deterministicTime = {};", env.logical_time);
        runtime.execute_script("[set_time]", time_code.into())
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to set time: {}", e)))?;
        
        // Reset dependency tracking
        runtime.execute_script("[reset]", "globalThis.__readKeys = new Set(); globalThis.__writtenKeys = new Set();".into())
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to reset: {}", e)))?;
        
        Ok(())
    }
    
    fn generate_deterministic_seed(tx_id: TxId) -> u64 {
        // Use transaction ID and WAL position to generate seed
        let bytes = tx_id.0.as_bytes();
        let mut seed = 0u64;
        for (i, &byte) in bytes.iter().enumerate().take(8) {
            seed |= (byte as u64) << (i * 8);
        }
        seed
    }
}