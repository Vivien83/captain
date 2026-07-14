//! Shared kernel handle access helper for tool handlers.

use crate::kernel_handle::KernelHandle;
use std::sync::Arc;

pub(crate) fn require_kernel(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<&Arc<dyn KernelHandle>, String> {
    kernel.ok_or_else(|| {
        "Kernel handle not available. Inter-agent tools require a running kernel.".to_string()
    })
}
