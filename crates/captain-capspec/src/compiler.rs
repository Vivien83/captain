use crate::model::{
    toml_to_json, CompiledCapability, CompiledStep, Effect, Idempotency, InputField, InputType,
    PermissionSet, SourceCapability, CAPABILITY_TOOL_PREFIX, CAPSPEC_FORMAT_VERSION,
};
use crate::policy::{
    reviewed_effect, validate_permissions, validate_scoped_permission, validate_tool_name,
};
use crate::template::template_references;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
const MAX_STEPS: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 512;
const MAX_CAPABILITY_NAME_CHARS: usize = 55;
const MAX_TIMEOUT_SECS: u64 = 3_600;
const MAX_PARALLEL: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("CapSpec source exceeds {MAX_SOURCE_BYTES} bytes")]
    SourceTooLarge,
    #[error("invalid CapSpec TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid CapSpec: {0}")]
    Invalid(String),
}

pub fn parse(source: &str) -> Result<SourceCapability, CompileError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(CompileError::SourceTooLarge);
    }
    Ok(toml::from_str(source)?)
}

pub fn compile(source: &str) -> Result<CompiledCapability, CompileError> {
    let source_capability = parse(source)?;
    compile_named(source, source_capability, None)
}

pub fn compile_named(
    source: &str,
    mut raw: SourceCapability,
    expected_file_stem: Option<&str>,
) -> Result<CompiledCapability, CompileError> {
    validate_header(&raw, expected_file_stem)?;
    raw.permissions.normalize();
    validate_permissions(&raw.permissions).map_err(CompileError::Invalid)?;
    validate_inputs(&raw.inputs)?;
    validate_permission_templates(&raw.permissions, &raw.inputs)?;
    validate_policy(&raw.policy)?;

    let source_hash = blake3::hash(source.as_bytes()).to_hex().to_string();
    let permission_fingerprint = raw.permissions.fingerprint();
    let input_schema = build_input_schema(&raw.inputs, raw.policy.allow_extra_inputs)?;
    let output = match &raw.output {
        Some(output) => toml_to_json(output).map_err(CompileError::Invalid)?,
        None => Value::String(format!(
            "{{{{steps.{}.output}}}}",
            raw.steps.last().expect("validated non-empty steps").id
        )),
    };
    let steps = compile_steps(&raw, &output)?;
    let tool_name = tool_name_for(&raw.name);

    Ok(CompiledCapability {
        format: raw.format,
        name: raw.name,
        tool_name,
        description: raw.description,
        version: raw.version,
        tags: normalized_tags(raw.tags),
        source_hash,
        permission_fingerprint,
        input_schema,
        inputs: raw.inputs,
        permissions: raw.permissions,
        policy: raw.policy,
        steps,
        output,
    })
}

fn validate_header(
    raw: &SourceCapability,
    expected_file_stem: Option<&str>,
) -> Result<(), CompileError> {
    if raw.format != CAPSPEC_FORMAT_VERSION {
        return invalid(format!(
            "unsupported format {}; expected {}",
            raw.format, CAPSPEC_FORMAT_VERSION
        ));
    }
    validate_identifier("capability name", &raw.name, MAX_CAPABILITY_NAME_CHARS)?;
    if raw.name.contains('_') {
        return invalid("capability name must use '-' instead of '_' to keep tool names unique");
    }
    if let Some(stem) = expected_file_stem {
        if stem != raw.name {
            return invalid(format!(
                "file stem '{stem}' must match capability name '{}'",
                raw.name
            ));
        }
    }
    let description_len = raw.description.trim().chars().count();
    if description_len == 0 || description_len > MAX_DESCRIPTION_CHARS {
        return invalid(format!(
            "description must contain 1..={MAX_DESCRIPTION_CHARS} characters"
        ));
    }
    if raw.version.trim().is_empty() || raw.version.len() > 64 {
        return invalid("version must contain 1..=64 characters");
    }
    if raw.steps.is_empty() || raw.steps.len() > MAX_STEPS {
        return invalid(format!("steps must contain 1..={MAX_STEPS} entries"));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str, max_chars: usize) -> Result<(), CompileError> {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() || chars.len() > max_chars {
        return invalid(format!("{label} must contain 1..={max_chars} characters"));
    }
    if !chars[0].is_ascii_lowercase() && !chars[0].is_ascii_digit() {
        return invalid(format!(
            "{label} must start with a lowercase letter or digit"
        ));
    }
    if chars
        .iter()
        .any(|ch| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_')))
    {
        return invalid(format!(
            "{label} may contain only lowercase ASCII letters, digits, '-' and '_'"
        ));
    }
    Ok(())
}

fn validate_permission_templates(
    permissions: &PermissionSet,
    inputs: &BTreeMap<String, InputField>,
) -> Result<(), CompileError> {
    let no_steps = HashSet::new();
    for scope in permissions
        .read_paths
        .iter()
        .chain(&permissions.write_paths)
        .chain(&permissions.network_hosts)
        .chain(&permissions.ssh_hosts)
        .chain(&permissions.shell_commands)
        .chain(&permissions.memory_read)
        .chain(&permissions.memory_write)
    {
        if scope.chars().any(|ch| ch.is_control()) {
            return invalid("permission scopes cannot contain control characters");
        }
        validate_template_references(&Value::String(scope.clone()), inputs, &no_steps, None)?;
    }
    Ok(())
}

fn validate_inputs(inputs: &BTreeMap<String, InputField>) -> Result<(), CompileError> {
    if inputs.len() > 64 {
        return invalid("inputs cannot contain more than 64 fields");
    }
    for (name, field) in inputs {
        validate_identifier("input name", name, 64)?;
        if field.description.chars().count() > 256 {
            return invalid(format!("input '{name}' description exceeds 256 characters"));
        }
        if field.input_type != InputType::Array && field.items.is_some() {
            return invalid(format!("input '{name}' uses items but is not an array"));
        }
        if let Some(default) = &field.default {
            let default = toml_to_json(default).map_err(CompileError::Invalid)?;
            if !field.input_type.accepts(&default) {
                return invalid(format!("input '{name}' default has the wrong type"));
            }
        }
        for value in &field.enum_values {
            let value = toml_to_json(value).map_err(CompileError::Invalid)?;
            if !field.input_type.accepts(&value) {
                return invalid(format!("input '{name}' enum contains the wrong type"));
            }
        }
    }
    Ok(())
}

fn validate_policy(policy: &crate::model::CapabilityPolicy) -> Result<(), CompileError> {
    if policy.timeout_secs == 0 || policy.timeout_secs > MAX_TIMEOUT_SECS {
        return invalid(format!(
            "policy.timeout_secs must be 1..={MAX_TIMEOUT_SECS}"
        ));
    }
    if policy.max_parallel == 0 || policy.max_parallel > MAX_PARALLEL {
        return invalid(format!("policy.max_parallel must be 1..={MAX_PARALLEL}"));
    }
    Ok(())
}

fn build_input_schema(
    inputs: &BTreeMap<String, InputField>,
    allow_extra_inputs: bool,
) -> Result<Value, CompileError> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, field) in inputs {
        let mut schema = Map::new();
        schema.insert("type".to_string(), json!(field.input_type.json_name()));
        if !field.description.trim().is_empty() {
            schema.insert("description".to_string(), json!(field.description));
        }
        if field.sensitive {
            schema.insert("writeOnly".to_string(), json!(true));
        }
        if let Some(default) = &field.default {
            schema.insert(
                "default".to_string(),
                toml_to_json(default).map_err(CompileError::Invalid)?,
            );
        }
        if !field.enum_values.is_empty() {
            let values = field
                .enum_values
                .iter()
                .map(toml_to_json)
                .collect::<Result<Vec<_>, _>>()
                .map_err(CompileError::Invalid)?;
            schema.insert("enum".to_string(), Value::Array(values));
        }
        if let Some(items) = field.items {
            schema.insert("items".to_string(), json!({"type": items.json_name()}));
        }
        if field.required && field.default.is_none() {
            required.push(Value::String(name.clone()));
        }
        properties.insert(name.clone(), Value::Object(schema));
    }
    Ok(json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": allow_extra_inputs,
    }))
}

fn compile_steps(
    raw: &SourceCapability,
    output: &Value,
) -> Result<Vec<CompiledStep>, CompileError> {
    let allowed_tools: HashSet<&str> = raw.permissions.tools.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut steps = Vec::with_capacity(raw.steps.len());

    for (index, step) in raw.steps.iter().enumerate() {
        validate_identifier("step id", &step.id, 64)?;
        if !seen.insert(step.id.clone()) {
            return invalid(format!("duplicate step id '{}'", step.id));
        }
        validate_tool_name(&step.tool).map_err(CompileError::Invalid)?;
        if !allowed_tools.contains(step.tool.as_str()) {
            return invalid(format!(
                "step '{}' calls '{}' which is absent from permissions.tools",
                step.id, step.tool
            ));
        }
        let needs = step.needs.clone().unwrap_or_else(|| {
            if index == 0 {
                Vec::new()
            } else {
                vec![raw.steps[index - 1].id.clone()]
            }
        });
        let minimum_effect = reviewed_effect(&step.tool);
        let effect = step.effect.unwrap_or(minimum_effect);
        if effect < minimum_effect {
            return invalid(format!(
                "step '{}' declares effect {:?}, below the {:?} minimum for '{}'",
                step.id, effect, minimum_effect, step.tool
            ));
        }
        validate_scoped_permission(&step.id, &step.tool, &raw.permissions)
            .map_err(CompileError::Invalid)?;

        let idempotency = step.idempotency.unwrap_or(if effect == Effect::Read {
            Idempotency::Safe
        } else {
            Idempotency::Manual
        });
        if effect != Effect::Read && idempotency == Idempotency::Safe {
            return invalid(format!(
                "step '{}' cannot claim safe idempotency for a non-read effect; use keyed or manual",
                step.id
            ));
        }
        if idempotency == Idempotency::Keyed
            && step
                .idempotency_key
                .as_deref()
                .is_none_or(|key| key.trim().is_empty())
        {
            return invalid(format!(
                "step '{}' uses keyed idempotency without idempotency_key",
                step.id
            ));
        }
        if step.retry.max_attempts == 0 || step.retry.max_attempts > 10 {
            return invalid(format!(
                "step '{}' retry.max_attempts must be 1..=10",
                step.id
            ));
        }
        if step.retry.max_attempts > 1 && idempotency == Idempotency::Manual {
            return invalid(format!(
                "step '{}' cannot retry with manual idempotency",
                step.id
            ));
        }
        let timeout_secs = step.timeout_secs.unwrap_or(raw.policy.timeout_secs);
        if timeout_secs == 0 || timeout_secs > raw.policy.timeout_secs {
            return invalid(format!(
                "step '{}' timeout must be 1..={} seconds",
                step.id, raw.policy.timeout_secs
            ));
        }
        let input = match &step.input {
            Some(value) => toml_to_json(value).map_err(CompileError::Invalid)?,
            None => json!({}),
        };
        if !input.is_object() {
            return invalid(format!(
                "step '{}' with must be a TOML inline table or table",
                step.id
            ));
        }

        steps.push(CompiledStep {
            id: step.id.clone(),
            tool: step.tool.clone(),
            needs,
            input,
            effect,
            idempotency,
            idempotency_key: step.idempotency_key.clone(),
            timeout_secs,
            retry: step.retry.clone(),
        });
    }

    validate_graph_and_templates(&steps, &raw.inputs, output)?;
    Ok(steps)
}

fn validate_graph_and_templates(
    steps: &[CompiledStep],
    inputs: &BTreeMap<String, InputField>,
    output: &Value,
) -> Result<(), CompileError> {
    let ids: HashSet<&str> = steps.iter().map(|step| step.id.as_str()).collect();
    for step in steps {
        for need in &step.needs {
            if need == &step.id {
                return invalid(format!("step '{}' cannot depend on itself", step.id));
            }
            if !ids.contains(need.as_str()) {
                return invalid(format!("step '{}' needs unknown step '{need}'", step.id));
            }
        }
    }
    let ancestors = topological_ancestors(steps)?;
    for step in steps {
        validate_template_references(
            &step.input,
            inputs,
            &ids,
            Some((&step.id, ancestors.get(&step.id).unwrap())),
        )?;
        if let Some(key) = &step.idempotency_key {
            validate_template_references(
                &Value::String(key.clone()),
                inputs,
                &ids,
                Some((&step.id, ancestors.get(&step.id).unwrap())),
            )?;
        }
    }
    validate_template_references(output, inputs, &ids, None)
}

fn topological_ancestors(
    steps: &[CompiledStep],
) -> Result<HashMap<String, HashSet<String>>, CompileError> {
    let mut indegree: HashMap<String, usize> = steps
        .iter()
        .map(|step| (step.id.clone(), step.needs.len()))
        .collect();
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for step in steps {
        for need in &step.needs {
            children
                .entry(need.clone())
                .or_default()
                .push(step.id.clone());
        }
    }
    let mut queue: VecDeque<String> = indegree
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut ancestors: HashMap<String, HashSet<String>> = steps
        .iter()
        .map(|step| (step.id.clone(), HashSet::new()))
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        for child in children.get(&id).into_iter().flatten() {
            let inherited = ancestors.get(&id).cloned().unwrap_or_default();
            let child_ancestors = ancestors.get_mut(child).unwrap();
            child_ancestors.insert(id.clone());
            child_ancestors.extend(inherited);
            let count = indegree.get_mut(child).unwrap();
            *count -= 1;
            if *count == 0 {
                queue.push_back(child.clone());
            }
        }
    }
    if visited != steps.len() {
        return invalid("step dependency graph contains a cycle");
    }
    Ok(ancestors)
}

fn validate_template_references(
    value: &Value,
    inputs: &BTreeMap<String, InputField>,
    step_ids: &HashSet<&str>,
    current_step: Option<(&str, &HashSet<String>)>,
) -> Result<(), CompileError> {
    for reference in
        template_references(value).map_err(|error| CompileError::Invalid(error.to_string()))?
    {
        if reference == "run.id" {
            continue;
        }
        if let Some(path) = reference.strip_prefix("input.") {
            let name = path.split('.').next().unwrap_or_default();
            if !inputs.contains_key(name) {
                return invalid(format!("template references unknown input '{name}'"));
            }
            continue;
        }
        if let Some(path) = reference.strip_prefix("steps.") {
            let mut parts = path.split('.');
            let step = parts.next().unwrap_or_default();
            if !step_ids.contains(step) || parts.next() != Some("output") {
                return invalid(format!(
                    "invalid step output reference '{{{{{reference}}}}}'"
                ));
            }
            if let Some((current, ancestors)) = current_step {
                if !ancestors.contains(step) {
                    return invalid(format!(
                        "step '{current}' references '{step}' without depending on it"
                    ));
                }
            }
            continue;
        }
        return invalid(format!(
            "unsupported template reference '{{{{{reference}}}}}'"
        ));
    }
    Ok(())
}

fn tool_name_for(name: &str) -> String {
    format!("{CAPABILITY_TOOL_PREFIX}{}", name.replace('-', "_"))
}

fn normalized_tags(tags: Vec<String>) -> Vec<String> {
    let tags: BTreeSet<String> = tags
        .into_iter()
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.into_iter().collect()
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CompileError> {
    Err(CompileError::Invalid(message.into()))
}

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;
