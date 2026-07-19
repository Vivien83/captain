use serde_json::{Map, Value};
use std::collections::BTreeMap;

const MAX_TEMPLATE_TOKENS: usize = 128;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("unclosed template token")]
    Unclosed,
    #[error("empty template token")]
    Empty,
    #[error("too many template tokens")]
    TooMany,
    #[error("unknown template reference: {0}")]
    UnknownReference(String),
    #[error("template reference {0} is not scalar and cannot be embedded in text")]
    NonScalar(String),
}

pub struct TemplateContext<'a> {
    pub run_id: &'a str,
    pub input: &'a Map<String, Value>,
    pub step_outputs: &'a BTreeMap<String, Value>,
}

pub fn template_references(value: &Value) -> Result<Vec<String>, TemplateError> {
    let mut references = Vec::new();
    collect_references(value, &mut references)?;
    if references.len() > MAX_TEMPLATE_TOKENS {
        return Err(TemplateError::TooMany);
    }
    Ok(references)
}

fn collect_references(value: &Value, output: &mut Vec<String>) -> Result<(), TemplateError> {
    match value {
        Value::String(text) => output.extend(tokens(text)?),
        Value::Array(values) => {
            for value in values {
                collect_references(value, output)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_references(value, output)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn tokens(text: &str) -> Result<Vec<String>, TemplateError> {
    let mut remaining = text;
    let mut found = Vec::new();
    while let Some(start) = remaining.find("{{") {
        let after = &remaining[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(TemplateError::Unclosed);
        };
        let token = after[..end].trim();
        if token.is_empty() {
            return Err(TemplateError::Empty);
        }
        found.push(token.to_string());
        remaining = &after[end + 2..];
    }
    Ok(found)
}

pub fn render_template(
    value: &Value,
    context: &TemplateContext<'_>,
) -> Result<Value, TemplateError> {
    match value {
        Value::String(text) => render_string(text, context),
        Value::Array(values) => values
            .iter()
            .map(|value| render_template(value, context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), render_template(value, context)?)))
            .collect::<Result<Map<_, _>, _>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}

fn render_string(text: &str, context: &TemplateContext<'_>) -> Result<Value, TemplateError> {
    let refs = tokens(text)?;
    if refs.is_empty() {
        return Ok(Value::String(text.to_string()));
    }
    let exact = refs.len() == 1 && text.trim() == format!("{{{{{}}}}}", refs[0]);
    if exact {
        if refs[0] == "run.id" {
            return Ok(Value::String(context.run_id.to_string()));
        }
        return resolve_reference(&refs[0], context).cloned();
    }

    let mut rendered = text.to_string();
    for reference in refs {
        let replacement = if reference == "run.id" {
            context.run_id.to_string()
        } else {
            let value = resolve_reference(&reference, context)?;
            scalar_text(value).ok_or_else(|| TemplateError::NonScalar(reference.clone()))?
        };
        rendered = rendered.replace(&format!("{{{{{reference}}}}}"), &replacement);
    }
    Ok(Value::String(rendered))
}

fn resolve_reference<'a>(
    reference: &str,
    context: &'a TemplateContext<'_>,
) -> Result<&'a Value, TemplateError> {
    if let Some(path) = reference.strip_prefix("input.") {
        return descend_map(context.input, path)
            .ok_or_else(|| TemplateError::UnknownReference(reference.to_string()));
    }
    if let Some(path) = reference.strip_prefix("steps.") {
        let mut parts = path.split('.');
        let Some(step) = parts.next() else {
            return Err(TemplateError::UnknownReference(reference.to_string()));
        };
        if parts.next() != Some("output") {
            return Err(TemplateError::UnknownReference(reference.to_string()));
        }
        let Some(mut value) = context.step_outputs.get(step) else {
            return Err(TemplateError::UnknownReference(reference.to_string()));
        };
        for part in parts {
            value = value
                .get(part)
                .ok_or_else(|| TemplateError::UnknownReference(reference.to_string()))?;
        }
        return Ok(value);
    }
    Err(TemplateError::UnknownReference(reference.to_string()))
}

fn descend_map<'a>(values: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut value = values.get(first)?;
    for part in parts {
        value = value.get(part)?;
    }
    Some(value)
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => Some("null".to_string()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context<'a>(
        input: &'a Map<String, Value>,
        step_outputs: &'a BTreeMap<String, Value>,
    ) -> TemplateContext<'a> {
        TemplateContext {
            run_id: "run-1",
            input,
            step_outputs,
        }
    }

    #[test]
    fn exact_reference_preserves_json_type() {
        let input = json!({"count": 3}).as_object().unwrap().clone();
        let steps = BTreeMap::new();
        let rendered =
            render_template(&json!("{{input.count}}"), &context(&input, &steps)).unwrap();
        assert_eq!(rendered, json!(3));
    }

    #[test]
    fn embedded_reference_requires_scalar() {
        let input = json!({"items": [1, 2]}).as_object().unwrap().clone();
        let steps = BTreeMap::new();
        let error =
            render_template(&json!("items={{input.items}}"), &context(&input, &steps)).unwrap_err();
        assert_eq!(error, TemplateError::NonScalar("input.items".to_string()));
    }

    #[test]
    fn step_output_can_select_nested_field() {
        let input = Map::new();
        let steps = BTreeMap::from([("fetch".to_string(), json!({"status": 200}))]);
        let rendered = render_template(
            &json!("status={{steps.fetch.output.status}}"),
            &context(&input, &steps),
        )
        .unwrap();
        assert_eq!(rendered, json!("status=200"));
    }

    #[test]
    fn run_id_renders_as_exact_or_embedded_text() {
        let input = Map::new();
        let steps = BTreeMap::new();
        let context = context(&input, &steps);
        assert_eq!(
            render_template(&json!("{{run.id}}"), &context).unwrap(),
            json!("run-1")
        );
        assert_eq!(
            render_template(&json!("key:{{run.id}}"), &context).unwrap(),
            json!("key:run-1")
        );
    }
}
