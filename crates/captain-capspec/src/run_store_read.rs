use super::*;
use rusqlite::OptionalExtension;

impl RunStore {
    pub(crate) fn load_run(&self, run_id: &str) -> Result<StoredRun, ExecutorError> {
        let row = self
            .connection
            .query_row(
                "SELECT scope_json, capability_name, tool_name, source_hash, input_blob,
                        status, output_blob, caller_agent_id, workspace,
                        origin, authority_blob, created_at, updated_at
                 FROM capspec_runs WHERE run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<Vec<u8>>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<Vec<u8>>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| ExecutorError::RunNotFound(run_id.to_string()))?;
        let input = self
            .open_json(&format!("run:{run_id}:input"), &row.4)?
            .as_object()
            .cloned()
            .ok_or_else(|| ExecutorError::InvalidState("run input is not an object".to_string()))?;
        let output = self.open_optional_json(&format!("run:{run_id}:output"), row.6)?;
        let authority = self
            .open_optional_json(&format!("run:{run_id}:authority"), row.10)?
            .map(serde_json::from_value)
            .transpose()?;
        let nodes = self.load_nodes(run_id)?;
        let view = CapabilityRunView {
            run_id: run_id.to_string(),
            scope: serde_json::from_str(&row.0)?,
            capability_name: row.1,
            tool_name: row.2,
            source_hash: row.3,
            status: CapabilityRunStatus::from_storage(&row.5)?,
            caller_agent_id: row.7,
            workspace: row.8,
            origin: row.9,
            created_at: row.11,
            updated_at: row.12,
            nodes: nodes.iter().map(StoredNode::view).collect(),
        };
        let execution = CapabilityExecutionContext {
            caller_agent_id: view.caller_agent_id.clone(),
            workspace: view.workspace.clone(),
            origin: view.origin.clone(),
            authority,
        };
        Ok(StoredRun {
            view,
            execution,
            input,
            output,
            nodes,
        })
    }

    pub(crate) fn list_runs(&self, limit: usize) -> Result<Vec<CapabilityRunView>, ExecutorError> {
        let mut statement = self.connection.prepare(
            "SELECT run_id FROM capspec_runs ORDER BY updated_at DESC, run_id DESC LIMIT ?1",
        )?;
        let ids = statement
            .query_map([limit.min(500) as i64], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.load_view(id)).collect()
    }

    pub(crate) fn list_waiting_runs(
        &self,
        limit: usize,
    ) -> Result<Vec<CapabilityRunView>, ExecutorError> {
        let mut statement = self.connection.prepare(
            "SELECT run_id FROM capspec_runs
             WHERE status = 'waiting_decision'
             ORDER BY updated_at DESC, run_id DESC LIMIT ?1",
        )?;
        let ids = statement
            .query_map([limit.clamp(1, 5_000) as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter().map(|id| self.load_view(id)).collect()
    }

    pub(crate) fn list_operator_resume_run_ids(
        &self,
        limit: usize,
    ) -> Result<Vec<String>, ExecutorError> {
        let mut statement = self.connection.prepare(
            "SELECT run_id FROM capspec_runs
             WHERE operator_resume_state IN ('requested', 'in_progress')
               AND status IN ('pending', 'running', 'interrupted')
             ORDER BY updated_at ASC, run_id ASC LIMIT ?1",
        )?;
        let ids = statement
            .query_map([limit.clamp(1, 5_000) as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    pub(crate) fn load_view(&self, run_id: &str) -> Result<CapabilityRunView, ExecutorError> {
        let row = self
            .connection
            .query_row(
                "SELECT scope_json, capability_name, tool_name, source_hash, status,
                        caller_agent_id, workspace, origin, created_at, updated_at
                 FROM capspec_runs WHERE run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| ExecutorError::RunNotFound(run_id.to_string()))?;
        Ok(CapabilityRunView {
            run_id: run_id.to_string(),
            scope: serde_json::from_str(&row.0)?,
            capability_name: row.1,
            tool_name: row.2,
            source_hash: row.3,
            status: CapabilityRunStatus::from_storage(&row.4)?,
            caller_agent_id: row.5,
            workspace: row.6,
            origin: row.7,
            created_at: row.8,
            updated_at: row.9,
            nodes: self.load_node_views(run_id)?,
        })
    }

    fn load_nodes(&self, run_id: &str) -> Result<Vec<StoredNode>, ExecutorError> {
        let mut statement = self.connection.prepare(
            "SELECT step_id, ordinal, tool_name, effect, status, attempts,
                    operator_retry_permit, output_blob, error_blob, tool_use_id,
                    started_at, finished_at
             FROM capspec_run_nodes WHERE run_id = ?1 ORDER BY ordinal",
        )?;
        let rows = statement
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, Option<Vec<u8>>>(7)?,
                    row.get::<_, Option<Vec<u8>>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|row| -> Result<StoredNode, ExecutorError> {
                let step_id = row.0;
                let prefix = format!("run:{run_id}:node:{step_id}");
                Ok(StoredNode {
                    step_id,
                    ordinal: row.1,
                    tool_name: row.2,
                    effect: parse_effect(&row.3)?,
                    status: CapabilityNodeStatus::from_storage(&row.4)?,
                    attempts: row.5,
                    operator_retry_permit: row.6,
                    output: self.open_optional_json(&format!("{prefix}:output"), row.7)?,
                    error: self.open_optional_text(&format!("{prefix}:error"), row.8)?,
                    tool_use_id: row.9,
                    started_at: row.10,
                    finished_at: row.11,
                })
            })
            .collect()
    }

    fn load_node_views(&self, run_id: &str) -> Result<Vec<CapabilityNodeView>, ExecutorError> {
        let mut statement = self.connection.prepare(
            "SELECT step_id, ordinal, tool_name, effect, status, attempts,
                    tool_use_id, started_at, finished_at
             FROM capspec_run_nodes WHERE run_id = ?1 ORDER BY ordinal",
        )?;
        let views = statement
            .query_map([run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, u32>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })?
            .map(|row| -> Result<CapabilityNodeView, ExecutorError> {
                let row = row?;
                Ok(CapabilityNodeView {
                    step_id: row.0,
                    ordinal: row.1,
                    tool_name: row.2,
                    effect: parse_effect(&row.3)?,
                    status: CapabilityNodeStatus::from_storage(&row.4)?,
                    attempts: row.5,
                    tool_use_id: row.6,
                    started_at: row.7,
                    finished_at: row.8,
                })
            })
            .collect::<Result<Vec<_>, ExecutorError>>()?;
        Ok(views)
    }
}

impl StoredNode {
    fn view(&self) -> CapabilityNodeView {
        CapabilityNodeView {
            step_id: self.step_id.clone(),
            ordinal: self.ordinal,
            tool_name: self.tool_name.clone(),
            effect: self.effect,
            status: self.status,
            attempts: self.attempts,
            tool_use_id: self.tool_use_id.clone(),
            started_at: self.started_at.clone(),
            finished_at: self.finished_at.clone(),
        }
    }
}

fn parse_effect(value: &str) -> Result<Effect, ExecutorError> {
    match value {
        "read" => Ok(Effect::Read),
        "write" => Ok(Effect::Write),
        "external" => Ok(Effect::External),
        "destructive" => Ok(Effect::Destructive),
        other => Err(ExecutorError::InvalidState(format!(
            "unknown effect '{other}'"
        ))),
    }
}
