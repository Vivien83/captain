//! WASM sandbox for secure skill/plugin execution.
//!
//! Uses Wasmtime to execute untrusted WASM modules with deny-by-default
//! capability-based permissions. No filesystem, network, or credential
//! access unless explicitly granted.
//!
//! # Guest ABI
//!
//! WASM modules must export:
//! - `memory` — linear memory
//! - `alloc(size: i32) -> i32` — allocate `size` bytes, return pointer
//! - `execute(input_ptr: i32, input_len: i32) -> i64` — main entry point
//!
//! The `execute` function receives JSON input bytes and returns a packed
//! `i64` value: `(result_ptr << 32) | result_len`. The result is JSON bytes.
//!
//! # Host ABI
//!
//! The host provides (in the `"captain"` import module):
//! - `host_call(request_ptr: i32, request_len: i32) -> i64` — RPC dispatch
//! - `host_log(level: i32, msg_ptr: i32, msg_len: i32)` — logging
//!
//! `host_call` reads a JSON request `{"method": "...", "params": {...}}`
//! and returns a packed pointer to JSON `{"ok": ...}` or `{"error": "..."}`.

use crate::host_functions;
use crate::kernel_handle::KernelHandle;
use captain_types::capability::Capability;
use std::sync::Arc;
use tracing::debug;
use wasmtime::*;

/// Configuration for a WASM sandbox instance.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Maximum fuel (CPU instruction budget). 0 = unlimited.
    pub fuel_limit: u64,
    /// Maximum WASM linear memory in bytes (reserved for future enforcement).
    pub max_memory_bytes: usize,
    /// Capabilities granted to this sandbox instance.
    pub capabilities: Vec<Capability>,
    /// Wall-clock timeout in seconds for epoch-based interruption.
    /// Defaults to 30 seconds if None.
    pub timeout_secs: Option<u64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            fuel_limit: 1_000_000,
            max_memory_bytes: 16 * 1024 * 1024,
            capabilities: Vec::new(),
            timeout_secs: None,
        }
    }
}

/// State carried in each WASM Store, accessible by host functions.
pub struct GuestState {
    /// Capabilities granted to this guest — checked before every host call.
    pub capabilities: Vec<Capability>,
    /// Handle to kernel for inter-agent operations.
    pub kernel: Option<Arc<dyn KernelHandle>>,
    /// Agent ID of the calling agent.
    pub agent_id: String,
    /// Tokio runtime handle for async operations in sync host functions.
    pub tokio_handle: tokio::runtime::Handle,
}

/// Result of executing a WASM module.
#[derive(Debug)]
pub struct ExecutionResult {
    /// JSON output from the guest's `execute` function.
    pub output: serde_json::Value,
    /// Number of fuel units consumed.
    pub fuel_consumed: u64,
}

/// Errors from sandbox operations.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("WASM compilation failed: {0}")]
    Compilation(String),
    #[error("WASM instantiation failed: {0}")]
    Instantiation(String),
    #[error("WASM execution failed: {0}")]
    Execution(String),
    #[error("Fuel exhausted: skill exceeded CPU budget")]
    FuelExhausted,
    #[error("Guest ABI violation: {0}")]
    AbiError(String),
}

/// The WASM sandbox engine.
///
/// Create one per kernel, reuse across skill invocations. The `Engine`
/// is expensive to create but can compile/instantiate many modules.
pub struct WasmSandbox {
    engine: Engine,
}

struct GuestExports {
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    execute: TypedFunc<(i32, i32), i64>,
}

struct GuestOutput {
    ptr: usize,
    len: usize,
}

struct HostCallRequest {
    method: String,
    params: serde_json::Value,
}

impl WasmSandbox {
    /// Create a new sandbox engine with fuel metering enabled.
    pub fn new() -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| SandboxError::Compilation(e.to_string()))?;
        Ok(Self { engine })
    }

    /// Execute a WASM module with the given JSON input.
    ///
    /// All host calls from within the module are subject to capability checks.
    /// Execution is offloaded to a blocking thread (CPU-bound WASM should not
    /// run on the Tokio executor).
    pub async fn execute(
        &self,
        wasm_bytes: &[u8],
        input: serde_json::Value,
        config: SandboxConfig,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: &str,
    ) -> Result<ExecutionResult, SandboxError> {
        let engine = self.engine.clone();
        let wasm_bytes = wasm_bytes.to_vec();
        let agent_id = agent_id.to_string();
        let handle = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            Self::execute_sync(
                &engine,
                &wasm_bytes,
                input,
                &config,
                kernel,
                &agent_id,
                handle,
            )
        })
        .await
        .map_err(|e| SandboxError::Execution(format!("spawn_blocking join failed: {e}")))?
    }

    /// Synchronous inner execution — runs on a blocking thread.
    fn execute_sync(
        engine: &Engine,
        wasm_bytes: &[u8],
        input: serde_json::Value,
        config: &SandboxConfig,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: &str,
        tokio_handle: tokio::runtime::Handle,
    ) -> Result<ExecutionResult, SandboxError> {
        let module = compile_guest_module(engine, wasm_bytes)?;
        let mut store = build_guest_store(engine, config, kernel, agent_id, tokio_handle)?;
        let timeout = config.timeout_secs.unwrap_or(30);
        let _watchdog = spawn_epoch_watchdog(engine, timeout);
        let instance = instantiate_guest_module(engine, &mut store, &module)?;
        let exports = guest_exports(&instance, &mut store)?;
        let (input_ptr, input_len) =
            write_guest_input(&mut store, &exports.memory, &exports.alloc, &input)?;
        let packed =
            call_guest_execute(&mut store, &exports.execute, input_ptr, input_len, timeout)?;
        let output = read_guest_output(&store, &exports.memory, unpack_guest_result(packed))?;
        let fuel_consumed = fuel_consumed(&store, config);

        debug!(agent = agent_id, fuel_consumed, "WASM execution complete");

        Ok(ExecutionResult {
            output,
            fuel_consumed,
        })
    }

    /// Register host function imports in the linker ("captain" module).
    fn register_host_functions(linker: &mut Linker<GuestState>) -> Result<(), SandboxError> {
        register_host_call(linker)?;
        register_host_log(linker)?;
        Ok(())
    }
}

fn compile_guest_module(engine: &Engine, wasm_bytes: &[u8]) -> Result<Module, SandboxError> {
    Module::new(engine, wasm_bytes).map_err(|e| SandboxError::Compilation(e.to_string()))
}

fn build_guest_store(
    engine: &Engine,
    config: &SandboxConfig,
    kernel: Option<Arc<dyn KernelHandle>>,
    agent_id: &str,
    tokio_handle: tokio::runtime::Handle,
) -> Result<Store<GuestState>, SandboxError> {
    let mut store = Store::new(
        engine,
        GuestState {
            capabilities: config.capabilities.clone(),
            kernel,
            agent_id: agent_id.to_string(),
            tokio_handle,
        },
    );
    if config.fuel_limit > 0 {
        store
            .set_fuel(config.fuel_limit)
            .map_err(|e| SandboxError::Execution(e.to_string()))?;
    }
    store.set_epoch_deadline(1);
    Ok(store)
}

fn spawn_epoch_watchdog(engine: &Engine, timeout: u64) -> std::thread::JoinHandle<()> {
    let engine_clone = engine.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(timeout));
        engine_clone.increment_epoch();
    })
}

fn instantiate_guest_module(
    engine: &Engine,
    store: &mut Store<GuestState>,
    module: &Module,
) -> Result<Instance, SandboxError> {
    let mut linker = Linker::new(engine);
    WasmSandbox::register_host_functions(&mut linker)?;
    linker
        .instantiate(store, module)
        .map_err(|e| SandboxError::Instantiation(e.to_string()))
}

fn guest_exports(
    instance: &Instance,
    store: &mut Store<GuestState>,
) -> Result<GuestExports, SandboxError> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| SandboxError::AbiError("Module must export 'memory'".into()))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "alloc")
        .map_err(|e| {
            SandboxError::AbiError(format!("Module must export 'alloc(i32)->i32': {e}"))
        })?;
    let execute = instance
        .get_typed_func::<(i32, i32), i64>(&mut *store, "execute")
        .map_err(|e| {
            SandboxError::AbiError(format!("Module must export 'execute(i32,i32)->i64': {e}"))
        })?;
    Ok(GuestExports {
        memory,
        alloc,
        execute,
    })
}

fn write_guest_input(
    store: &mut Store<GuestState>,
    memory: &Memory,
    alloc_fn: &TypedFunc<i32, i32>,
    input: &serde_json::Value,
) -> Result<(i32, i32), SandboxError> {
    let input_bytes = serde_json::to_vec(input)
        .map_err(|e| SandboxError::Execution(format!("JSON serialize failed: {e}")))?;
    let input_len = input_bytes.len() as i32;
    let input_ptr = alloc_fn
        .call(&mut *store, input_len)
        .map_err(|e| SandboxError::AbiError(format!("alloc call failed: {e}")))?;
    let mem_data = memory.data_mut(&mut *store);
    let start = input_ptr as usize;
    let end = start + input_bytes.len();
    if end > mem_data.len() {
        return Err(SandboxError::AbiError("Input exceeds memory bounds".into()));
    }
    mem_data[start..end].copy_from_slice(&input_bytes);
    Ok((input_ptr, input_len))
}

fn call_guest_execute(
    store: &mut Store<GuestState>,
    execute_fn: &TypedFunc<(i32, i32), i64>,
    input_ptr: i32,
    input_len: i32,
    timeout: u64,
) -> Result<i64, SandboxError> {
    execute_fn
        .call(&mut *store, (input_ptr, input_len))
        .map_err(|e| {
            if let Some(Trap::OutOfFuel) = e.downcast_ref::<Trap>() {
                return SandboxError::FuelExhausted;
            }
            if let Some(Trap::Interrupt) = e.downcast_ref::<Trap>() {
                return SandboxError::Execution(format!(
                    "WASM execution timed out after {}s (epoch interrupt)",
                    timeout
                ));
            }
            SandboxError::Execution(e.to_string())
        })
}

fn unpack_guest_result(packed: i64) -> GuestOutput {
    GuestOutput {
        ptr: (packed >> 32) as usize,
        len: (packed & 0xFFFF_FFFF) as usize,
    }
}

fn read_guest_output(
    store: &Store<GuestState>,
    memory: &Memory,
    result: GuestOutput,
) -> Result<serde_json::Value, SandboxError> {
    let mem_data = memory.data(store);
    if result.ptr + result.len > mem_data.len() {
        return Err(SandboxError::AbiError(
            "Result pointer out of bounds".into(),
        ));
    }
    serde_json::from_slice(&mem_data[result.ptr..result.ptr + result.len])
        .map_err(|e| SandboxError::AbiError(format!("Invalid JSON output from guest: {e}")))
}

fn fuel_consumed(store: &Store<GuestState>, config: &SandboxConfig) -> u64 {
    let fuel_remaining = store.get_fuel().unwrap_or(0);
    config.fuel_limit.saturating_sub(fuel_remaining)
}

fn register_host_call(linker: &mut Linker<GuestState>) -> Result<(), SandboxError> {
    linker
        .func_wrap(
            "captain",
            "host_call",
            |mut caller: Caller<'_, GuestState>,
             request_ptr: i32,
             request_len: i32|
             -> Result<i64, wasmtime::Error> {
                handle_host_call(&mut caller, request_ptr, request_len)
                    .map_err(wasmtime::Error::from_anyhow)
            },
        )
        .map_err(|e| SandboxError::Compilation(e.to_string()))?;
    Ok(())
}

fn handle_host_call(
    caller: &mut Caller<'_, GuestState>,
    request_ptr: i32,
    request_len: i32,
) -> Result<i64, anyhow::Error> {
    let request_bytes = read_guest_bytes(
        caller,
        request_ptr,
        request_len,
        "host_call: request out of bounds",
    )?;
    let request = parse_host_call_request(&request_bytes)?;
    let response = host_functions::dispatch(caller.data(), &request.method, &request.params);
    write_host_response(caller, &response)
}

fn parse_host_call_request(request_bytes: &[u8]) -> Result<HostCallRequest, anyhow::Error> {
    let request: serde_json::Value = serde_json::from_slice(request_bytes)?;
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let params = request
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(HostCallRequest { method, params })
}

fn write_host_response(
    caller: &mut Caller<'_, GuestState>,
    response: &serde_json::Value,
) -> Result<i64, anyhow::Error> {
    let response_bytes = serde_json::to_vec(response)?;
    let len = response_bytes.len() as i32;
    let alloc_fn = caller
        .get_export("alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| anyhow::anyhow!("no alloc export"))?;
    let alloc_typed = alloc_fn.typed::<i32, i32>(&mut *caller)?;
    let ptr = alloc_typed.call(&mut *caller, len)?;
    let memory = caller_memory(caller)?;
    let mem_data = memory.data_mut(&mut *caller);
    let dest_start = ptr as usize;
    let dest_end = dest_start + response_bytes.len();
    if dest_end > mem_data.len() {
        anyhow::bail!("host_call: response exceeds memory bounds");
    }
    mem_data[dest_start..dest_end].copy_from_slice(&response_bytes);
    Ok(pack_guest_result(ptr, len))
}

fn register_host_log(linker: &mut Linker<GuestState>) -> Result<(), SandboxError> {
    linker
        .func_wrap(
            "captain",
            "host_log",
            |mut caller: Caller<'_, GuestState>,
             level: i32,
             msg_ptr: i32,
             msg_len: i32|
             -> Result<(), wasmtime::Error> {
                handle_host_log(&mut caller, level, msg_ptr, msg_len)
                    .map_err(wasmtime::Error::from_anyhow)
            },
        )
        .map_err(|e| SandboxError::Compilation(e.to_string()))?;
    Ok(())
}

fn handle_host_log(
    caller: &mut Caller<'_, GuestState>,
    level: i32,
    msg_ptr: i32,
    msg_len: i32,
) -> Result<(), anyhow::Error> {
    let msg_bytes = read_guest_bytes(caller, msg_ptr, msg_len, "host_log: pointer out of bounds")?;
    let msg = std::str::from_utf8(&msg_bytes).unwrap_or("<invalid utf8>");
    let agent_id = &caller.data().agent_id;
    log_guest_message(level, agent_id, msg);
    Ok(())
}

fn read_guest_bytes(
    caller: &mut Caller<'_, GuestState>,
    ptr: i32,
    len: i32,
    out_of_bounds: &str,
) -> Result<Vec<u8>, anyhow::Error> {
    let memory = caller_memory(caller)?;
    let data = memory.data(&mut *caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        anyhow::bail!(out_of_bounds.to_string());
    }
    Ok(data[start..end].to_vec())
}

fn caller_memory(caller: &mut Caller<'_, GuestState>) -> Result<Memory, anyhow::Error> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| anyhow::anyhow!("no memory export"))
}

fn log_guest_message(level: i32, agent_id: &str, msg: &str) {
    match level {
        0 => tracing::trace!(agent = %agent_id, "[wasm] {msg}"),
        1 => tracing::debug!(agent = %agent_id, "[wasm] {msg}"),
        2 => tracing::info!(agent = %agent_id, "[wasm] {msg}"),
        3 => tracing::warn!(agent = %agent_id, "[wasm] {msg}"),
        _ => tracing::error!(agent = %agent_id, "[wasm] {msg}"),
    }
}

fn pack_guest_result(ptr: i32, len: i32) -> i64 {
    ((ptr as i64) << 32) | (len as i64)
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
