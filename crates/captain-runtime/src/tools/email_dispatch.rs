//! Native Gmail tool dispatch.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_email_accounts, tool_email_attachment_save, tool_email_automation_deliveries,
    tool_email_automation_delivery_requeue, tool_email_automation_rule_remove,
    tool_email_automation_rule_save, tool_email_automation_rule_set_enabled,
    tool_email_automation_rules, tool_email_compose, tool_email_labels, tool_email_read,
    tool_email_reply, tool_email_search, tool_email_update,
};

pub(crate) async fn dispatch_email_tool(
    tool_name: &str,
    input: &Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    match tool_name {
        "email_accounts" => tool_email_accounts(input, kernel),
        "email_search" => tool_email_search(input, kernel).await,
        "email_read" => tool_email_read(input, kernel).await,
        "email_compose" => tool_email_compose(input, kernel, workspace_root).await,
        "email_reply" => tool_email_reply(input, kernel, workspace_root).await,
        "email_labels" => tool_email_labels(input, kernel).await,
        "email_update" => tool_email_update(input, kernel).await,
        "email_attachment_save" => tool_email_attachment_save(input, kernel, workspace_root).await,
        "email_automation_rules" => tool_email_automation_rules(input, kernel),
        "email_automation_rule_save" => tool_email_automation_rule_save(input, kernel),
        "email_automation_rule_set_enabled" => {
            tool_email_automation_rule_set_enabled(input, kernel)
        }
        "email_automation_rule_remove" => tool_email_automation_rule_remove(input, kernel),
        "email_automation_deliveries" => tool_email_automation_deliveries(input, kernel),
        "email_automation_delivery_requeue" => {
            tool_email_automation_delivery_requeue(input, kernel)
        }
        other => Err(format!("Unknown email tool: {other}")),
    }
}
