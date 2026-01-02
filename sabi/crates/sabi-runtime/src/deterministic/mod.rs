//! Deterministic execution environment
//!
//! Rules:
//! - No system randomness
//! - No wall-clock time
//! - No external I/O
//! - All operations must be deterministic given same inputs

/*
 deterministic JavaScript runtime - basically a sandboxed JavaScript environment where the 
 same inputs ALWAYS produce the same outputs, no matter when or where you run it.

 This is crucial because we need to ensure that every time a function is executed, it behaves exactly the same way,
 even if the function is run multiple times in different contexts or at different times.
 This means we have to control things like random number generation, time, and any side effects that
 might affect the outcome of the function.
 This is achieved by replacing non-deterministic APIs with deterministic versions,
 and by ensuring that all operations are logged in a way that allows us to replay them exactly.

 This code takes a regular JavaScript engine (Deno/V8) and removes all the "non-deterministic" parts - things that could give different results each time:

    Random numbers
    Current time/date
    External I/O (network calls, file reads)
*/

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
    /// Arc makes this Thread-safe. Mutex ensures only one thread can access at a time  
    runtime: Arc<Mutex<JsRuntime>>,
    
    /// Environment for deterministic operations
    env: Arc<Mutex<HashMap<TxId, DeterministicEnv>>>,
    
    /// Global function registry
    functions: Arc<Mutex<HashMap<String, String>>>,
}

impl DeterministicRuntime {
    pub fn new() -> Result<Self, RuntimeError> {
        // let mut runtime = JsRuntime::new(RuntimeOptions {
        //     // will_snapshot: true,
        //     extensions: vec![
        //         // Register deterministic APIs
        //         deno_core::Extension::builder()
        //             .js(deno_core::include_js_files!(
        //                 "deterministic_apis",
        //                 "src/deterministic/apis.js",
        //             ))
        //             .build(),
        //     ],
        //     ..Default::default()
        // });

        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![
                // Register deterministic APIs
                deno_core::Extension {
                    name: "deterministic_apis",
                    js_files: std::borrow::Cow::Borrowed(&[
                        deno_core::ExtensionFileSource {
                            specifier: "ext:deterministic_apis/apis.js",
                            code: deno_core::ExtensionFileSourceCode::IncludedInBinary(
                                include_str!("apis.js")
                            ),
                        }
                    ]),
                    ..Default::default()
                },
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

        // This code replaces the default Math.random and Date.now with deterministic versions
        // and sets up a global database context for queries and mutations.
        // It also ensures that all read/write operations are tracked for determinism.
        // The code is executed in the JavaScript runtime to ensure it runs in the correct context
        // and can access the global scope.
        // This is crucial for ensuring that all operations are deterministic and can be replayed
        // exactly the same way every time.

        /*
        
        This replaces the built-in random function with a pseudo-random generator. Give it the same seed, 
        get the same "random" sequence every time. It uses the Xorshift algorithm - a simple but effective way to generate pseudo-random numbers.

        Math.random = function() {
            _randomSeed ^= _randomSeed << 13;  // Xorshift algorithm
            _randomSeed ^= _randomSeed >> 7;
            _randomSeed ^= _randomSeed << 17;
            return (_randomSeed >>> 0) / 4294967296;
        };
            ============================================


        Instead of returning the actual current time, it returns a controlled "logical time" that you set. 
        This means Date.now() gives you 0 (or whatever you set) instead of the real timestamp.

        Date.now = function() {
            return globalThis.__deterministicTime || 0;
        };

        ============================================

        globalThis.db = {
            query: (q, ...args) => { /* ... */ },
            get: (key) => {
                globalThis.__recordRead(key);  // Track what was read
                return globalThis.__storageGet(key);
            },
            put: (key, value) => {
                globalThis.__recordWrite(key);  // Track what was written
                globalThis.__storagePut(key, value);
            }
        };

        This creates a custom database API that tracks dependencies - which keys your function reads from or writes to. This is crucial for:

        Caching (if inputs haven't changed, skip re-execution)
        Conflict detection (did two transactions touch the same data?)
        Replay (re-run transactions in the right order)
         */
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
                globalThis.__recordRead(key); // Track what was read
                return globalThis.__storageGet(key);
            },
            // Write operations - go through WAL
            put: (key, value) => {
                globalThis.__recordWrite(key); // Track what was written
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
            // insert deterministic context
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
        
        // Get function source code
        let functions = self.functions.lock()?;
        let source = functions.get(function_name)
            .ok_or_else(|| RuntimeError::InvalidFunction(function_name.to_string()))?;
        
        // Execute in JavaScript runtime
        let mut runtime = self.runtime.lock()?;
        
        // Set up execution environment
        self.prepare_runtime(&mut runtime, env, &args)?;
        
        // Execute the function
        // Convert Rust Values to JSON
        // If args = [JsValue::Number(42), JsValue::String("hello")]
        // Then js_args = [JsonValue::Number(42), JsonValue::String("hello")]
        let js_args: Vec<JsonValue> = args.iter()
            .map(|v| serde_json::to_value(v).unwrap())
            .collect();
        
        // Serialize to a JSON String
        // js_args_str = "[42, "hello"]"
        let js_args_str = serde_json::to_string(&js_args)
            .map_err(RuntimeError::SerializationError)?;
        
        /*
        The {} are placeholders in the format! macro that get replaced with: 
        First {} → source (the function code) Second {} → js_args_str (the arguments as a JSON string)

        const result = ({})(...{});
         */
        let js_code = format!(
            r#"
            try {{
                globalThis.__readKeys = new Set();
                globalThis.__writtenKeys = new Set();
                
                // Execute the function
                // This is an IIFE (Immediately Invoked Function Expression)
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
        
        // execute_script(...) => Sends the code to V8 (the JavaScript engine)
        let result = runtime.execute_script(&format!("[{}]", function_name), js_code.into())
            .map_err(|e| RuntimeError::FunctionError(format!("Execution failed: {}", e)))?;
        
        // allow all pending ops to complete
        // poll_event_loop(false, None) ==> false = Don't wait indefinitely, None = No timeout
        let result_value = runtime.poll_event_loop(false, None)
            .map_err(|e| RuntimeError::FunctionError(format!("Event loop failed: {}", e)))?;
        
        // Extract result from V8
        //  Get a V8 Scope; V8 uses "handles" to manage JavaScript values (for garbage collection); A "scope" is like a context where these handles are valid
        let scope = &mut runtime.handle_scope();
        let local_result = v8::Local::new(scope, result);

        // Deserialize from V8 to JSON
        let result_json: JsonValue = serde_v8::from_v8(scope, local_result)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to deserialize result: {}", e)))?;
        
        /*
        Expected Json structure:
        {
            "value": <the actual result>,
            "readKeys": ["key1", "key2"],
            "writtenKeys": ["key3"]
        } 

        or 

        {
            "error": "Something went wrong"
        }
         */
        // Parse result
        if let Some(error) = result_json.get("error") {
            return Err(RuntimeError::FunctionError(error.as_str().unwrap().to_string()));
        }
        
        // result_json must have "value", "readKeys", "writtenKeys
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
        env: DeterministicEnv, // The deterministic environment to apply
        args: &[JsValue], 
    ) -> Result<(), RuntimeError> {
        // Set deterministic random seed here needed when executing the function
        let seed_code = format!("_randomSeed = {};", env.random_seed);
        // run code to set the random seed
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