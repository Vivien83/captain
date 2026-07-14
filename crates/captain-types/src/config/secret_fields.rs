//! Structural guards for config fields that hold secret material.

struct DirectSecretPath {
    label: &'static str,
    segments: &'static [&'static str],
}

const DIRECT_SECRET_PATHS: &[DirectSecretPath] = &[
    DirectSecretPath {
        label: "api_key",
        segments: &["api_key"],
    },
    DirectSecretPath {
        label: "api.api_key",
        segments: &["api", "api_key"],
    },
    DirectSecretPath {
        label: "network.shared_secret",
        segments: &["network", "shared_secret"],
    },
    DirectSecretPath {
        label: "auth.password_hash",
        segments: &["auth", "password_hash"],
    },
];

pub fn find_direct_secret_assignments(source: &str) -> Result<Vec<&'static str>, toml::de::Error> {
    let value: toml::Value = source.parse()?;
    Ok(find_direct_secret_assignments_in_value(&value))
}

pub fn find_direct_secret_assignments_in_value(value: &toml::Value) -> Vec<&'static str> {
    DIRECT_SECRET_PATHS
        .iter()
        .filter_map(|path| {
            let value = lookup_toml_path(value, path.segments)?;
            toml_value_is_set(value).then_some(path.label)
        })
        .collect()
}

fn lookup_toml_path<'a>(value: &'a toml::Value, segments: &[&str]) -> Option<&'a toml::Value> {
    let mut current = value;
    for segment in segments {
        current = current.as_table()?.get(*segment)?;
    }
    Some(current)
}

fn toml_value_is_set(value: &toml::Value) -> bool {
    match value {
        toml::Value::String(value) => !value.trim().is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_non_empty_direct_secret_fields() {
        let raw = r#"
api_key = "captain_api_secret"

[api]
api_key = "legacy_api_secret"

[network]
shared_secret = "ofp-shared-secret"

[auth]
password_hash = "$argon2id$v=19$m=4096,t=3,p=1$hash"
"#;

        let leaks = find_direct_secret_assignments(raw).unwrap();
        assert_eq!(
            leaks,
            vec![
                "api_key",
                "api.api_key",
                "network.shared_secret",
                "auth.password_hash"
            ]
        );
    }

    #[test]
    fn allows_empty_secret_fields_and_env_references() {
        let raw = r#"
api_key = ""

[default_model]
api_key_env = "OPENAI_API_KEY"

[network]
shared_secret = ""

[auth]
password_hash = ""
"#;

        let leaks = find_direct_secret_assignments(raw).unwrap();
        assert!(
            leaks.is_empty(),
            "unexpected direct secret fields: {leaks:?}"
        );
    }

    #[test]
    fn treats_non_string_direct_secret_fields_as_set() {
        let raw = r#"
api_key = 123
"#;

        let leaks = find_direct_secret_assignments(raw).unwrap();
        assert_eq!(leaks, vec!["api_key"]);
    }
}
