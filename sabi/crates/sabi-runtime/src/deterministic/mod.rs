//! Deterministic execution environment
//!
//! Rules:
//! - No system randomness
//! - No wall-clock time
//! - No external I/O
//! - All operations must be deterministic given same inputs
//!
//! Architecture:
//! The JsRuntime is not Send+Sync, so we run it on a dedicated thread.
//! Other threads communicate via channels to execute functions.

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender, Receiver};
use std::thread::{self, JoinHandle};
use std::borrow::Cow;

use serde_json::Value as JsonValue;
use deno_core::{JsRuntime, PollEventLoopOptions, RuntimeOptions, serde_v8, v8};
use futures::executor::block_on;
use sabi_core::TxId;

use crate::types::{JsValue, DeterministicEnv, FunctionResult, TransactionContext};
use crate::error::RuntimeError;

/// Commands sent to the runtime worker thread
enum RuntimeCommand {
    RegisterFunction {
        name: String,
        source: String,
        response: oneshot::Sender<Result<(), RuntimeError>>,
    },
    ExecuteQuery {
        tx_id: TxId,
        function_name: String,
        args: Vec<JsValue>,
        response: oneshot::Sender<Result<FunctionResult, RuntimeError>>,
    },
    ExecuteMutation {
        tx_id: TxId,
        function_name: String,
        args: Vec<JsValue>,
        response: oneshot::Sender<Result<FunctionResult, RuntimeError>>,
    },
    ExecuteAction {
        tx_id: TxId,
        function_name: String,
        args: Vec<JsValue>,
        response: oneshot::Sender<Result<FunctionResult, RuntimeError>>,
    },
    Shutdown,
}

/// Simple oneshot channel for responses
mod oneshot {
    use std::sync::mpsc;

    pub struct Sender<T>(mpsc::Sender<T>);
    pub struct Receiver<T>(mpsc::Receiver<T>);

    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let (tx, rx) = mpsc::channel();
        (Sender(tx), Receiver(rx))
    }

    impl<T> Sender<T> {
        pub fn send(self, value: T) -> Result<(), T> {
            self.0.send(value).map_err(|e| e.0)
        }
    }

    impl<T> Receiver<T> {
        pub fn recv(self) -> Result<T, mpsc::RecvError> {
            self.0.recv()
        }
    }
}

/// The worker that owns the JsRuntime and runs on a dedicated thread
struct RuntimeWorker {
    runtime: JsRuntime,
    env: HashMap<TxId, DeterministicEnv>,
    functions: HashMap<String, String>,
}

impl RuntimeWorker {
    fn new() -> Result<Self, RuntimeError> {
        let extension = deno_core::Extension {
            name: "deterministic_apis",
            deps: &[],
            js_files: Cow::Borrowed(&[]),
            esm_files: Cow::Borrowed(&[]),
            lazy_loaded_esm_files: Cow::Borrowed(&[]),
            esm_entry_point: None,
            ops: Cow::Borrowed(&[]),
            objects: Cow::Borrowed(&[]),
            external_references: Cow::Borrowed(&[]),
            global_template_middleware: None,
            global_object_middleware: None,
            op_state_fn: None,
            needs_lazy_init: false,
            middleware_fn: None,
            enabled: true,
        };

        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions: vec![extension],
            ..Default::default()
        });

        // Load JavaScript file if it exists
        if let Ok(js_code) = std::fs::read_to_string("src/deterministic/apis.js") {
            runtime.execute_script("deterministic_apis.js", js_code)
                .map_err(|e| RuntimeError::FunctionError(format!("Failed to load apis.js: {}", e)))?;
        }

        // Initialize deterministic APIs
        Self::init_deterministic_apis(&mut runtime)?;

        Ok(Self {
            runtime,
            env: HashMap::new(),
            functions: HashMap::new(),
        })
    }

    fn init_deterministic_apis(runtime: &mut JsRuntime) -> Result<(), RuntimeError> {
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
            get: (key) => {
                globalThis.__recordRead(key);
                return globalThis.__storageGet(key);
            },
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

        runtime.execute_script("[deterministic]", code)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to init APIs: {}", e)))?;

        Ok(())
    }

    fn run(mut self, receiver: Receiver<RuntimeCommand>) {
        while let Ok(command) = receiver.recv() {
            match command {
                RuntimeCommand::RegisterFunction { name, source, response } => {
                    let result = self.register_function(&name, &source);
                    let _ = response.send(result);
                }
                RuntimeCommand::ExecuteQuery { tx_id, function_name, args, response } => {
                    let result = self.execute_query(tx_id, &function_name, args);
                    let _ = response.send(result);
                }
                RuntimeCommand::ExecuteMutation { tx_id, function_name, args, response } => {
                    let result = self.execute_mutation(tx_id, &function_name, args);
                    let _ = response.send(result);
                }
                RuntimeCommand::ExecuteAction { tx_id, function_name, args, response } => {
                    let result = self.execute_action(tx_id, &function_name, args);
                    let _ = response.send(result);
                }
                RuntimeCommand::Shutdown => {
                    break;
                }
            }
        }
    }

    fn register_function(&mut self, name: &str, source: &str) -> Result<(), RuntimeError> {
        self.functions.insert(name.to_string(), source.to_string());
        Ok(())
    }

    fn execute_query(
        &mut self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, true);
        self.execute_function("query", function_name, args, env)
    }

    fn execute_mutation(
        &mut self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, false);
        self.execute_function("mutation", function_name, args, env)
    }

    fn execute_action(
        &mut self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let env = self.create_env(tx_id, false);
        self.execute_function("action", function_name, args, env)
    }

    fn create_env(&mut self, tx_id: TxId, is_read_only: bool) -> DeterministicEnv {
        if !self.env.contains_key(&tx_id) {
            let seed = Self::generate_deterministic_seed(tx_id);
            let context = TransactionContext {
                tx_id: tx_id.0,
                start_time: 0,
                is_read_only,
            };

            self.env.insert(tx_id, DeterministicEnv {
                random_seed: seed,
                logical_time: 0,
                context,
            });
        }

        self.env[&tx_id].clone()
    }

    fn execute_function(
        &mut self,
        function_type: &str,
        function_name: &str,
        args: Vec<JsValue>,
        env: DeterministicEnv,
    ) -> Result<FunctionResult, RuntimeError> {
        let start_time = std::time::Instant::now();

        // Get function source code
        let source = self.functions.get(function_name)
            .ok_or_else(|| RuntimeError::InvalidFunction(function_name.to_string()))?
            .clone();

        // Set up execution environment
        self.prepare_runtime(&env)?;

        // Convert args to JSON
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

                const result = ({})(...{});

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

        let script_name = format!("[{}:{}]", function_type, function_name);
        let static_name: &'static str = Box::leak(script_name.into_boxed_str());

        let result = self.runtime.execute_script(static_name, js_code)
            .map_err(|e| RuntimeError::FunctionError(format!("Execution failed: {}", e)))?;

        // Run event loop to completion
        let _ = block_on(self.runtime.run_event_loop(PollEventLoopOptions {
            wait_for_inspector: false,
            pump_v8_message_loop: true,
        }));

        // Extract result from V8
        let scope = &mut self.runtime.handle_scope();
        let local_result = v8::Local::new(scope, result);

        let result_json: JsonValue = serde_v8::from_v8(scope, local_result)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to deserialize result: {}", e)))?;

        // Parse result
        if let Some(error) = result_json.get("error") {
            return Err(RuntimeError::FunctionError(
                error.as_str().unwrap_or("Unknown error").to_string()
            ));
        }

        let value = serde_json::from_value(
            result_json.get("value").cloned().unwrap_or(JsonValue::Null)
        ).map_err(RuntimeError::SerializationError)?;

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

    fn prepare_runtime(&mut self, env: &DeterministicEnv) -> Result<(), RuntimeError> {
        // Set deterministic random seed
        let seed_code = format!("_randomSeed = {};", env.random_seed);
        self.runtime.execute_script("[set_seed]", seed_code)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to set seed: {}", e)))?;

        // Set deterministic time
        let time_code = format!("globalThis.__deterministicTime = {};", env.logical_time);
        self.runtime.execute_script("[set_time]", time_code)
            .map_err(|e| RuntimeError::FunctionError(format!("Failed to set time: {}", e)))?;

        // Reset dependency tracking
        self.runtime.execute_script(
            "[reset]",
            "globalThis.__readKeys = new Set(); globalThis.__writtenKeys = new Set();"
        ).map_err(|e| RuntimeError::FunctionError(format!("Failed to reset: {}", e)))?;

        Ok(())
    }

    fn generate_deterministic_seed(tx_id: TxId) -> u64 {
        let bytes = tx_id.0.as_bytes();
        let mut seed = 0u64;
        for (i, &byte) in bytes.iter().enumerate().take(8) {
            seed |= (byte as u64) << (i * 8);
        }
        seed
    }
}

/// Thread-safe handle to the deterministic runtime.
///
/// This can be cloned and shared across threads. All JavaScript execution
/// happens on a dedicated worker thread, with commands sent via channels.
pub struct DeterministicRuntime {
    sender: Sender<RuntimeCommand>,
    _worker_handle: Option<JoinHandle<()>>,
}

// Safety: The sender is Send+Sync, and the JoinHandle is only used for cleanup
unsafe impl Send for DeterministicRuntime {}
unsafe impl Sync for DeterministicRuntime {}

impl DeterministicRuntime {
    /// Create a new deterministic runtime.
    ///
    /// This spawns a dedicated thread for JavaScript execution.
    pub fn new() -> Result<Self, RuntimeError> {
        let (sender, receiver) = mpsc::channel();

        // Create worker on dedicated thread
        let worker_handle = thread::Builder::new()
            .name("js-runtime".to_string())
            .spawn(move || {
                match RuntimeWorker::new() {
                    Ok(worker) => worker.run(receiver),
                    Err(e) => {
                        eprintln!("Failed to create JS runtime worker: {}", e);
                    }
                }
            })
            .map_err(|e| RuntimeError::IoError(format!("Failed to spawn runtime thread: {}", e)))?;

        Ok(Self {
            sender,
            _worker_handle: Some(worker_handle),
        })
    }

    /// Register a function for later execution
    pub fn register_function(&self, name: &str, source: &str) -> Result<(), RuntimeError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.sender.send(RuntimeCommand::RegisterFunction {
            name: name.to_string(),
            source: source.to_string(),
            response: response_tx,
        }).map_err(|_| RuntimeError::FunctionError("Runtime thread disconnected".to_string()))?;

        response_rx.recv()
            .map_err(|_| RuntimeError::FunctionError("Failed to receive response".to_string()))?
    }

    /// Execute a query function
    pub fn execute_query(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.sender.send(RuntimeCommand::ExecuteQuery {
            tx_id,
            function_name: function_name.to_string(),
            args,
            response: response_tx,
        }).map_err(|_| RuntimeError::FunctionError("Runtime thread disconnected".to_string()))?;

        response_rx.recv()
            .map_err(|_| RuntimeError::FunctionError("Failed to receive response".to_string()))?
    }

    /// Execute a mutation function
    pub fn execute_mutation(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.sender.send(RuntimeCommand::ExecuteMutation {
            tx_id,
            function_name: function_name.to_string(),
            args,
            response: response_tx,
        }).map_err(|_| RuntimeError::FunctionError("Runtime thread disconnected".to_string()))?;

        response_rx.recv()
            .map_err(|_| RuntimeError::FunctionError("Failed to receive response".to_string()))?
    }

    /// Execute an action function (read-write with external effects)
    pub fn execute_action(
        &self,
        tx_id: TxId,
        function_name: &str,
        args: Vec<JsValue>,
    ) -> Result<FunctionResult, RuntimeError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.sender.send(RuntimeCommand::ExecuteAction {
            tx_id,
            function_name: function_name.to_string(),
            args,
            response: response_tx,
        }).map_err(|_| RuntimeError::FunctionError("Runtime thread disconnected".to_string()))?;

        response_rx.recv()
            .map_err(|_| RuntimeError::FunctionError("Failed to receive response".to_string()))?
    }

    /// Shutdown the runtime worker thread
    pub fn shutdown(&self) {
        let _ = self.sender.send(RuntimeCommand::Shutdown);
    }
}

impl Drop for DeterministicRuntime {
    fn drop(&mut self) {
        self.shutdown();
        // Wait for worker thread to finish
        if let Some(handle) = self._worker_handle.take() {
            let _ = handle.join();
        }
    }
}
