use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) fn tool_peer_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    kh.list_external_agents()
}
