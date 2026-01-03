// Deterministic APIs exposed to user functions

// Record a key read (for dependency tracking)
globalThis.__recordRead = function(key) {
    if (globalThis.__readKeys) {
        globalThis.__readKeys.add(key);
    }
};

// Record a key write
globalThis.__recordWrite = function(key) {
    if (globalThis.__writtenKeys) {
        globalThis.__writtenKeys.add(key);
    }
};

// Storage operations - these would be implemented in Rust
globalThis.__storageGet = function(key) {
    // Implemented in Rust via FFI
    throw new Error("__storageGet must be implemented in Rust");
};

globalThis.__storagePut = function(key, value) {
    throw new Error("__storagePut must be implemented in Rust");
};

globalThis.__storageDelete = function(key) {
    throw new Error("__storageDelete must be implemented in Rust");
};

// Query execution
globalThis.__executeQuery = function(queryName, args) {
    throw new Error("__executeQuery must be implemented in Rust");
};

// Mutation execution
globalThis.__executeMutation = function(mutationName, args) {
    throw new Error("__executeMutation must be implemented in Rust");
};

// Prohibit non-deterministic APIs
Object.defineProperty(globalThis, 'crypto', {
    value: undefined,
    writable: false,
    configurable: false,
});

Object.defineProperty(globalThis, 'performance', {
    value: undefined,
    writable: false,
    configurable: false,
});

// Deterministic console (buffered, logged to WAL)
const originalConsole = console;
globalThis.console = {
    log: (...args) => {
        globalThis.__logToWal('log', args);
        originalConsole.log('[DET]', ...args);
    },
    error: (...args) => {
        globalThis.__logToWal('error', args);
        originalConsole.error('[DET]', ...args);
    },
    warn: (...args) => {
        globalThis.__logToWal('warn', args);
        originalConsole.warn('[DET]', ...args);
    },
    info: (...args) => {
        globalThis.__logToWal('info', args);
        originalConsole.info('[DET]', ...args);
    },
};

globalThis.__logToWal = function(level, args) {
    // Log to WAL for replay
    throw new Error("__logToWal must be implemented in Rust");
};