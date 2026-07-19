use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const CAPSPEC_FORMAT_VERSION: u32 = 1;
pub const CAPABILITY_TOOL_PREFIX: &str = "cap_";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCapability {
    pub format: u32,
    pub name: String,
    pub description: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputField>,
    #[serde(default)]
    pub permissions: PermissionSet,
    #[serde(default)]
    pub policy: CapabilityPolicy,
    pub steps: Vec<SourceStep>,
    #[serde(default)]
    pub output: Option<toml::Value>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputField {
    #[serde(rename = "type")]
    pub input_type: InputType,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub default: Option<toml::Value>,
    #[serde(default, rename = "enum")]
    pub enum_values: Vec<toml::Value>,
    #[serde(default)]
    pub items: Option<InputType>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    String,
    Integer,
    Number,
    Boolean,
    Object,
    Array,
}

impl InputType {
    pub fn json_name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::Array => "array",
        }
    }

    pub fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::Array => value.is_array(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionSet {
    pub tools: Vec<String>,
    pub read_paths: Vec<String>,
    pub write_paths: Vec<String>,
    pub network_hosts: Vec<String>,
    pub ssh_hosts: Vec<String>,
    pub shell_commands: Vec<String>,
    pub memory_read: Vec<String>,
    pub memory_write: Vec<String>,
    pub secrets: Vec<String>,
}

impl PermissionSet {
    pub fn normalize(&mut self) {
        normalize_values(&mut self.tools);
        normalize_values(&mut self.read_paths);
        normalize_values(&mut self.write_paths);
        normalize_values(&mut self.network_hosts);
        normalize_values(&mut self.ssh_hosts);
        normalize_values(&mut self.shell_commands);
        normalize_values(&mut self.memory_read);
        normalize_values(&mut self.memory_write);
        normalize_values(&mut self.secrets);
    }

    pub fn requires_human_approval(&self) -> bool {
        !self.write_paths.is_empty()
            || !self.network_hosts.is_empty()
            || !self.ssh_hosts.is_empty()
            || !self.shell_commands.is_empty()
            || !self.memory_write.is_empty()
            || !self.secrets.is_empty()
    }

    pub fn is_subset_of(&self, parent: &Self) -> bool {
        subset(&self.tools, &parent.tools)
            && subset(&self.read_paths, &parent.read_paths)
            && subset(&self.write_paths, &parent.write_paths)
            && subset(&self.network_hosts, &parent.network_hosts)
            && subset(&self.ssh_hosts, &parent.ssh_hosts)
            && subset(&self.shell_commands, &parent.shell_commands)
            && subset(&self.memory_read, &parent.memory_read)
            && subset(&self.memory_write, &parent.memory_write)
            && subset(&self.secrets, &parent.secrets)
    }

    pub fn fingerprint(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        blake3::hash(&bytes).to_hex().to_string()
    }
}

fn normalize_values(values: &mut Vec<String>) {
    let unique: BTreeSet<String> = values
        .drain(..)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect();
    values.extend(unique);
}

fn subset(child: &[String], parent: &[String]) -> bool {
    child
        .iter()
        .all(|value| parent.iter().any(|allowed| allowed == value))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CapabilityPolicy {
    pub timeout_secs: u64,
    pub max_parallel: usize,
    pub allow_extra_inputs: bool,
}

impl Default for CapabilityPolicy {
    fn default() -> Self {
        Self {
            timeout_secs: 300,
            max_parallel: 4,
            allow_extra_inputs: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceStep {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub needs: Option<Vec<String>>,
    #[serde(default, rename = "with")]
    pub input: Option<toml::Value>,
    #[serde(default)]
    pub effect: Option<Effect>,
    #[serde(default)]
    pub idempotency: Option<Idempotency>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub retry: RetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Read,
    Write,
    External,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Idempotency {
    Safe,
    Keyed,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledCapability {
    pub format: u32,
    pub name: String,
    pub tool_name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub source_hash: String,
    pub permission_fingerprint: String,
    pub input_schema: Value,
    pub inputs: BTreeMap<String, InputField>,
    pub permissions: PermissionSet,
    pub policy: CapabilityPolicy,
    pub steps: Vec<CompiledStep>,
    pub output: Value,
}

impl CompiledCapability {
    pub fn tool_definition(&self) -> captain_types::tool::ToolDefinition {
        captain_types::tool::ToolDefinition {
            name: self.tool_name.clone(),
            description: format!(
                "[CAPSPEC] {} (version {}, source {}).",
                self.description,
                self.version,
                &self.source_hash[..12]
            ),
            input_schema: self.input_schema.clone(),
        }
    }

    pub fn requires_human_approval(&self) -> bool {
        self.permissions.requires_human_approval()
            || self.steps.iter().any(|step| step.effect != Effect::Read)
    }

    pub fn validate_input(&self, input: &Value) -> Result<Map<String, Value>, String> {
        let object = input
            .as_object()
            .ok_or_else(|| "capability input must be a JSON object".to_string())?;
        if !self.policy.allow_extra_inputs {
            for key in object.keys() {
                if !self.inputs.contains_key(key) {
                    return Err(format!("unknown capability input '{key}'"));
                }
            }
        }

        let mut normalized = object.clone();
        for (name, field) in &self.inputs {
            let value = match normalized.get(name) {
                Some(value) => value,
                None => {
                    if let Some(default) = &field.default {
                        normalized.insert(name.clone(), toml_to_json(default)?);
                        continue;
                    }
                    if field.required {
                        return Err(format!("missing required capability input '{name}'"));
                    }
                    continue;
                }
            };
            if !field.input_type.accepts(value) {
                return Err(format!(
                    "capability input '{name}' must be {}",
                    field.input_type.json_name()
                ));
            }
            if !field.enum_values.is_empty() {
                let allowed = field
                    .enum_values
                    .iter()
                    .map(toml_to_json)
                    .collect::<Result<Vec<_>, _>>()?;
                if !allowed.contains(value) {
                    return Err(format!("capability input '{name}' is outside its enum"));
                }
            }
        }
        Ok(normalized)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledStep {
    pub id: String,
    pub tool: String,
    pub needs: Vec<String>,
    pub input: Value,
    pub effect: Effect,
    pub idempotency: Idempotency,
    pub idempotency_key: Option<String>,
    pub timeout_secs: u64,
    pub retry: RetryPolicy,
}

pub(crate) fn toml_to_json(value: &toml::Value) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("cannot convert TOML value to JSON: {error}"))
}
