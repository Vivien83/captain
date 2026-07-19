//! Captain Forge source contracts.
//!
//! A `.captain` file is parsed and compiled once into a typed, deterministic
//! execution plan. Runtime code consumes only the compiled plan; it never asks
//! an LLM to reinterpret source prose while a capability is running.

mod compiler;
mod executor;
mod executor_scope;
mod executor_types;
mod model;
mod policy;
mod registry;
mod registry_types;
mod run_store;
mod state_cipher;
mod store;
mod template;
mod watcher;

pub use compiler::{compile, compile_named, parse, CompileError, MAX_SOURCE_BYTES};
pub use executor::CapabilityExecutor;
pub use executor_types::{
    CapabilityExecution, CapabilityExecutionAuthority, CapabilityExecutionContext,
    CapabilityInvocation, CapabilityInvocationResult, CapabilityNodeStatus, CapabilityNodeView,
    CapabilityResumeContext, CapabilityRunStatus, CapabilityRunView, CapabilityToolInvoker,
    ExecutorError, UncertainNodeExpectation, UncertainResolution, UncertainResolutionReceipt,
};
pub use model::{
    CapabilityPolicy, CompiledCapability, CompiledStep, Effect, Idempotency, InputField, InputType,
    PermissionSet, RetryPolicy, SourceCapability, SourceStep, CAPABILITY_TOOL_PREFIX,
    CAPSPEC_FORMAT_VERSION,
};
pub use policy::reviewed_effect;
pub use registry::{CapabilityRegistry, ResolvedCapability};
pub use registry_types::{
    CapabilityScope, CapabilityStatus, CapabilityView, RegistryError, ReloadIssue, ReloadReport,
    RevisionInfo,
};
pub use template::{render_template, template_references, TemplateContext, TemplateError};
pub use watcher::{CapabilityWatcher, CapabilityWatcherStatus};
