//! Canonical action-graph construction for workflow learning evidence.

use std::collections::{BTreeMap, BTreeSet};

use captain_memory::workflow_learning::WorkflowEpisodeStepRecord;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::workflow_learning_analysis::{
    CanonicalWorkflow, CanonicalWorkflowNode, WorkflowRejectionReason,
};

const SIGNATURE_VERSION: u16 = 1;

#[derive(Clone)]
struct RawNode {
    id: String,
    ordinal: u32,
    tool_name: String,
    role: String,
    input_shape: Value,
    effect_class: String,
    verification_shape: String,
    dependencies: Vec<String>,
}

pub(crate) fn canonicalize_episode(
    steps: &[WorkflowEpisodeStepRecord],
) -> Result<CanonicalWorkflow, WorkflowRejectionReason> {
    let mut nodes = BTreeMap::new();
    for step in steps {
        let tool_name = normalize_tool_name(&step.tool_name)
            .ok_or(WorkflowRejectionReason::MalformedEvidence)?;
        let dependencies: Vec<String> = serde_json::from_str(&step.dependency_ids_json)
            .map_err(|_| WorkflowRejectionReason::MalformedEvidence)?;
        let input_shape: Value = serde_json::from_str(&step.input_shape_json)
            .map_err(|_| WorkflowRejectionReason::MalformedEvidence)?;
        if !input_shape.is_object() {
            return Err(WorkflowRejectionReason::MalformedEvidence);
        }
        let node = RawNode {
            id: step.tool_use_id.clone(),
            ordinal: step.ordinal,
            role: tool_role(&tool_name).to_string(),
            tool_name,
            input_shape: sort_json(input_shape),
            effect_class: step.effect_class.clone(),
            verification_shape: verification_shape(step),
            dependencies,
        };
        if nodes.insert(node.id.clone(), node).is_some() {
            return Err(WorkflowRejectionReason::MalformedEvidence);
        }
    }
    validate_full_graph(&nodes)?;

    let mut retained = nodes
        .iter()
        .filter_map(|(id, node)| (!transparent_role(&node.role)).then_some(id.clone()))
        .collect::<BTreeSet<_>>();
    if retained.is_empty() {
        retained.extend(
            nodes
                .iter()
                .filter_map(|(id, node)| (node.role == "memory").then_some(id.clone())),
        );
    }
    if retained.is_empty() {
        return Err(WorkflowRejectionReason::NoActionableSteps);
    }

    let mut expanded_dependencies = BTreeMap::new();
    for id in &retained {
        let node = nodes
            .get(id)
            .ok_or(WorkflowRejectionReason::MalformedEvidence)?;
        let mut dependencies = BTreeSet::new();
        for dependency in &node.dependencies {
            expand_retained_dependencies(
                dependency,
                &nodes,
                &retained,
                &mut BTreeSet::new(),
                &mut dependencies,
            )?;
        }
        dependencies.remove(id);
        expanded_dependencies.insert(id.clone(), dependencies);
    }

    let order = canonical_topological_order(&nodes, &retained, &expanded_dependencies)?;
    let indexes = order
        .iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let canonical_nodes = order
        .iter()
        .map(|id| {
            let node = &nodes[id];
            let dependencies = expanded_dependencies[id]
                .iter()
                .map(|dependency| indexes[dependency])
                .collect();
            CanonicalWorkflowNode {
                index: indexes[id],
                tool_name: node.tool_name.clone(),
                role: node.role.clone(),
                input_shape: node.input_shape.clone(),
                effect_class: node.effect_class.clone(),
                verification_shape: node.verification_shape.clone(),
                dependencies,
            }
        })
        .collect();

    Ok(CanonicalWorkflow {
        version: SIGNATURE_VERSION,
        nodes: canonical_nodes,
    })
}

fn validate_full_graph(nodes: &BTreeMap<String, RawNode>) -> Result<(), WorkflowRejectionReason> {
    let mut indegree = nodes
        .keys()
        .map(|id| (id.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, node) in nodes {
        let unique_dependencies = node.dependencies.iter().collect::<BTreeSet<_>>();
        for dependency in unique_dependencies {
            if dependency == id || !nodes.contains_key(dependency) {
                return Err(WorkflowRejectionReason::MalformedEvidence);
            }
            *indegree.get_mut(id).expect("known workflow node") += 1;
            adjacency
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect::<Vec<_>>();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        if let Some(children) = adjacency.get(&id) {
            for child in children {
                let degree = indegree.get_mut(child).expect("known workflow child");
                *degree -= 1;
                if *degree == 0 {
                    ready.push(child.clone());
                }
            }
        }
    }
    if visited == nodes.len() {
        Ok(())
    } else {
        Err(WorkflowRejectionReason::MalformedEvidence)
    }
}

fn expand_retained_dependencies(
    id: &str,
    nodes: &BTreeMap<String, RawNode>,
    retained: &BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
    output: &mut BTreeSet<String>,
) -> Result<(), WorkflowRejectionReason> {
    if retained.contains(id) {
        output.insert(id.to_string());
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        return Err(WorkflowRejectionReason::MalformedEvidence);
    }
    let node = nodes
        .get(id)
        .ok_or(WorkflowRejectionReason::MalformedEvidence)?;
    for dependency in &node.dependencies {
        expand_retained_dependencies(dependency, nodes, retained, visiting, output)?;
    }
    visiting.remove(id);
    Ok(())
}

fn canonical_topological_order(
    nodes: &BTreeMap<String, RawNode>,
    retained: &BTreeSet<String>,
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<String>, WorkflowRejectionReason> {
    let labels = retained
        .iter()
        .map(|id| (id.clone(), raw_node_label(&nodes[id])))
        .collect::<BTreeMap<_, _>>();
    let mut indegree = dependencies
        .iter()
        .map(|(id, dependencies)| (id.clone(), dependencies.len()))
        .collect::<BTreeMap<_, _>>();
    let mut adjacency: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (id, node_dependencies) in dependencies {
        for dependency in node_dependencies {
            adjacency
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(id.clone()))
        .collect::<Vec<_>>();
    let mut order = Vec::new();
    while !ready.is_empty() {
        ready.sort_by(|left, right| {
            labels[left]
                .cmp(&labels[right])
                .then_with(|| nodes[left].ordinal.cmp(&nodes[right].ordinal))
                .then_with(|| left.cmp(right))
        });
        let id = ready.remove(0);
        order.push(id.clone());
        if let Some(children) = adjacency.get(&id) {
            for child in children {
                let degree = indegree.get_mut(child).expect("known retained child");
                *degree -= 1;
                if *degree == 0 {
                    ready.push(child.clone());
                }
            }
        }
    }
    if order.len() == retained.len() {
        Ok(order)
    } else {
        Err(WorkflowRejectionReason::MalformedEvidence)
    }
}

pub(crate) fn tool_role(tool_name: &str) -> &'static str {
    if tool_name.starts_with("memory_") || tool_name.starts_with("knowledge_") {
        "memory"
    } else if matches!(
        tool_name,
        "tool_search"
            | "capability_search"
            | "captain_docs"
            | "skill_search"
            | "skill_view"
            | "session_recall"
            | "agent_find"
    ) {
        "discovery"
    } else if tool_name.starts_with("cron_")
        || tool_name.starts_with("schedule_")
        || tool_name.starts_with("file_trigger_")
        || tool_name.contains("webhook_trigger")
    {
        "automation"
    } else if matches!(tool_name, "web_search" | "web_research_batch") {
        "research"
    } else if tool_name.starts_with("browser_") {
        "browser"
    } else if tool_name.starts_with("agent_") {
        "delegation"
    } else if tool_name == "ask_user" {
        "human_input"
    } else if matches!(tool_name, "web_fetch" | "http_request" | "api_request") {
        "network_read"
    } else if tool_name.starts_with("ssh_health") {
        "remote_health"
    } else if tool_name.starts_with("ssh_") {
        "remote_command"
    } else if matches!(
        tool_name,
        "shell_exec" | "execute_code" | "cargo" | "npm" | "pip" | "docker_exec" | "docker_run"
    ) {
        "command"
    } else if tool_name.starts_with("git_") {
        "version_control"
    } else if matches!(tool_name, "document_extract" | "document_pipeline") {
        "document"
    } else if matches!(
        tool_name,
        "file_read" | "file_list" | "file_inspect_batch" | "grep" | "glob"
    ) {
        "file_read"
    } else if matches!(
        tool_name,
        "file_write" | "apply_patch" | "edit_file" | "multi_edit"
    ) {
        "file_write"
    } else if tool_name.starts_with("channel_")
        || tool_name.starts_with("email_")
        || tool_name.starts_with("telegram_")
        || tool_name.starts_with("discord_")
        || tool_name.starts_with("signal_")
    {
        "integration"
    } else {
        "integration"
    }
}

fn normalize_tool_name(tool_name: &str) -> Option<String> {
    let normalized = tool_name.trim().to_ascii_lowercase();
    (!normalized.is_empty()
        && normalized.len() <= 128
        && normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')))
    .then_some(normalized)
}

fn transparent_role(role: &str) -> bool {
    matches!(role, "memory" | "discovery")
}

fn verification_shape(step: &WorkflowEpisodeStepRecord) -> String {
    match step.verification_marker.as_deref() {
        Some("result_received") => "result_received".to_string(),
        Some("operation_confirmed") => "operation_confirmed".to_string(),
        Some("verified_by_read") => "verified_by_read".to_string(),
        Some(_) => "verified".to_string(),
        None if step.effect_class == "read" => "result_received".to_string(),
        None => "unverified".to_string(),
    }
}

fn raw_node_label(node: &RawNode) -> String {
    serde_json::to_string(&(
        &node.tool_name,
        &node.role,
        &node.input_shape,
        &node.effect_class,
        &node.verification_shape,
    ))
    .expect("normalized workflow node serializes")
}

pub(crate) fn canonical_signature(canonical: &CanonicalWorkflow) -> String {
    let mut node_hashes = Vec::with_capacity(canonical.nodes.len());
    for node in &canonical.nodes {
        let mut dependency_hashes = node
            .dependencies
            .iter()
            .map(|index| {
                node_hashes
                    .get(*index as usize)
                    .cloned()
                    .unwrap_or_else(|| "<invalid-dependency>".to_string())
            })
            .collect::<Vec<String>>();
        dependency_hashes.sort_unstable();
        let bytes = serde_json::to_vec(&(
            &node.tool_name,
            &node.role,
            &node.input_shape,
            &node.effect_class,
            &node.verification_shape,
            dependency_hashes,
        ))
        .expect("canonical workflow node serializes");
        node_hashes.push(format!("{:x}", Sha256::digest(bytes)));
    }
    node_hashes.sort_unstable();
    let bytes = serde_json::to_vec(&(SIGNATURE_VERSION, node_hashes))
        .expect("canonical workflow signature serializes");
    format!("{:x}", Sha256::digest(bytes))
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        other => other,
    }
}
