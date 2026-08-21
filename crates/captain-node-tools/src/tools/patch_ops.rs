//! Workspace patch application handler.

use crate::kernel_handle::KernelHandle;
use crate::tools::ensure_no_secret_literal;
use std::path::Path;
use std::sync::Arc;

pub(crate) async fn tool_apply_patch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    _kernel: Option<&Arc<dyn KernelHandle>>,
    _caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = workspace_root.ok_or("apply_patch requires a workspace root")?;
    let ops = crate::apply_patch::parse_patch(patch_str)?;
    ensure_patch_additions_have_no_secret_literals(&ops)?;
    let result = crate::apply_patch::apply_patch(&ops, root).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        Err(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        ))
    }
}

fn ensure_patch_additions_have_no_secret_literals(
    ops: &[crate::apply_patch::PatchOp],
) -> Result<(), String> {
    for (op_idx, op) in ops.iter().enumerate() {
        match op {
            crate::apply_patch::PatchOp::AddFile { content, .. } => {
                ensure_no_secret_literal("apply_patch", &format!("ops[{op_idx}].content"), content)?
            }
            crate::apply_patch::PatchOp::UpdateFile { hunks, .. } => {
                for (hunk_idx, hunk) in hunks.iter().enumerate() {
                    let added = hunk.new_lines.join("\n");
                    ensure_no_secret_literal(
                        "apply_patch",
                        &format!("ops[{op_idx}].hunks[{hunk_idx}].new_lines"),
                        &added,
                    )?;
                }
            }
            crate::apply_patch::PatchOp::DeleteFile { .. } => {}
        }
    }
    Ok(())
}
