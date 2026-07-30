use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{ExtensionError, ExtensionResult};

pub const SECRET_SOURCES_FILENAME: &str = "secret-sources.toml";
const SECRET_SOURCES_SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_SOURCES: usize = 256;
const MAX_SECRET_BYTES: u64 = 64 * 1024;
const MAX_KEY_BYTES: usize = 128;

#[derive(Debug, Clone, Default)]
pub struct ExternalSecretSources {
    sources: BTreeMap<String, ExternalSecretSource>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ExternalSecretSource {
    File { path: PathBuf },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretSourcesDocument {
    version: u32,
    #[serde(default)]
    sources: BTreeMap<String, ExternalSecretSource>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExternalSecretSourceStatus {
    pub key: String,
    pub source_type: String,
    pub ready: bool,
    pub authoritative: bool,
    pub live_rotation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSecretReadError {
    code: &'static str,
}

impl ExternalSecretReadError {
    fn new(code: &'static str) -> Self {
        Self { code }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

struct ExternalSecretValue {
    value: Zeroizing<String>,
    warning_code: Option<&'static str>,
}

impl ExternalSecretSources {
    pub fn load(path: &Path) -> ExtensionResult<Self> {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(ExtensionError::SecretSource(format!(
                    "could not inspect {SECRET_SOURCES_FILENAME}: {error}"
                )))
            }
        };
        let metadata = file.metadata().map_err(|error| {
            ExtensionError::SecretSource(format!(
                "could not inspect {SECRET_SOURCES_FILENAME}: {error}"
            ))
        })?;
        if !metadata.is_file() {
            return Err(ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} must be a regular file"
            )));
        }
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} exceeds {MAX_CONFIG_BYTES} bytes"
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(ExtensionError::SecretSource(format!(
                    "{SECRET_SOURCES_FILENAME} must not be writable by group or others"
                )));
            }
        }

        let mut bytes = Vec::with_capacity(metadata.len().min(MAX_CONFIG_BYTES) as usize);
        (&mut file)
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                ExtensionError::SecretSource(format!(
                    "could not read {SECRET_SOURCES_FILENAME}: {error}"
                ))
            })?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} exceeds {MAX_CONFIG_BYTES} bytes"
            )));
        }
        let raw = String::from_utf8(bytes).map_err(|_| {
            ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} must contain UTF-8 text"
            ))
        })?;
        let document: SecretSourcesDocument = toml::from_str(&raw).map_err(|error| {
            let location = error
                .span()
                .map(|span| format!(" near bytes {}..{}", span.start, span.end))
                .unwrap_or_default();
            ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} is invalid TOML{location}"
            ))
        })?;
        if document.version != SECRET_SOURCES_SCHEMA_VERSION {
            return Err(ExtensionError::SecretSource(format!(
                "unsupported {SECRET_SOURCES_FILENAME} version {}; expected {}",
                document.version, SECRET_SOURCES_SCHEMA_VERSION
            )));
        }
        if document.sources.len() > MAX_SOURCES {
            return Err(ExtensionError::SecretSource(format!(
                "{SECRET_SOURCES_FILENAME} defines more than {MAX_SOURCES} sources"
            )));
        }
        for (key, source) in &document.sources {
            validate_key(key)?;
            match source {
                ExternalSecretSource::File { path } if !path.is_absolute() => {
                    return Err(ExtensionError::SecretSource(format!(
                        "source '{key}' must use an absolute file path"
                    )))
                }
                ExternalSecretSource::File { .. } => {}
            }
        }

        Ok(Self {
            sources: document.sources,
        })
    }

    pub fn is_configured(&self, key: &str) -> bool {
        self.sources.contains_key(key)
    }

    pub fn configured_keys(&self) -> impl Iterator<Item = &str> {
        self.sources.keys().map(String::as_str)
    }

    /// Paths that must remain inaccessible to generic agent file tools.
    ///
    /// These paths are intentionally available only to trusted runtime code;
    /// statuses and serialized API responses never include them.
    pub fn protected_paths(&self) -> impl Iterator<Item = &Path> {
        self.sources.values().map(|source| match source {
            ExternalSecretSource::File { path } => path.as_path(),
        })
    }

    pub fn resolve(&self, key: &str) -> Result<Option<Zeroizing<String>>, ExternalSecretReadError> {
        let Some(source) = self.sources.get(key) else {
            return Ok(None);
        };
        read_source(source).map(|resolved| Some(resolved.value))
    }

    pub fn statuses(&self) -> Vec<ExternalSecretSourceStatus> {
        self.sources
            .iter()
            .map(|(key, source)| match read_source(source) {
                Ok(value) => ExternalSecretSourceStatus {
                    key: key.clone(),
                    source_type: source.kind().to_string(),
                    ready: true,
                    authoritative: true,
                    live_rotation: true,
                    error_code: None,
                    warning_code: value.warning_code.map(str::to_string),
                },
                Err(error) => ExternalSecretSourceStatus {
                    key: key.clone(),
                    source_type: source.kind().to_string(),
                    ready: false,
                    authoritative: true,
                    live_rotation: true,
                    error_code: Some(error.code().to_string()),
                    warning_code: None,
                },
            })
            .collect()
    }
}

impl ExternalSecretSource {
    fn kind(&self) -> &'static str {
        match self {
            Self::File { .. } => "file",
        }
    }
}

fn validate_key(key: &str) -> ExtensionResult<()> {
    let valid = !key.is_empty()
        && key.len() <= MAX_KEY_BYTES
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        });
    if valid {
        Ok(())
    } else {
        Err(ExtensionError::SecretSource(format!(
            "source key must be 1..={MAX_KEY_BYTES} bytes using env-var format: A-Z, 0-9, underscore"
        )))
    }
}

fn read_source(
    source: &ExternalSecretSource,
) -> Result<ExternalSecretValue, ExternalSecretReadError> {
    match source {
        ExternalSecretSource::File { path } => read_file_source(path),
    }
}

fn read_file_source(path: &Path) -> Result<ExternalSecretValue, ExternalSecretReadError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ExternalSecretReadError::new("source_missing")
        } else {
            ExternalSecretReadError::new("source_unreadable")
        }
    })?;
    let metadata = file
        .metadata()
        .map_err(|_| ExternalSecretReadError::new("source_unreadable"))?;
    if !metadata.is_file() {
        return Err(ExternalSecretReadError::new("source_not_regular_file"));
    }
    if metadata.len() > MAX_SECRET_BYTES {
        return Err(ExternalSecretReadError::new("source_too_large"));
    }

    #[cfg(unix)]
    let warning_code = {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        if mode & 0o022 != 0 {
            return Err(ExternalSecretReadError::new("source_writable_by_others"));
        }
        (mode & 0o044 != 0).then_some("source_readable_by_others")
    };
    #[cfg(not(unix))]
    let warning_code = None;

    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_SECRET_BYTES) as usize);
    (&mut file)
        .take(MAX_SECRET_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ExternalSecretReadError::new("source_unreadable"))?;
    if bytes.len() as u64 > MAX_SECRET_BYTES {
        return Err(ExternalSecretReadError::new("source_too_large"));
    }
    let text = Zeroizing::new(
        String::from_utf8(bytes).map_err(|_| ExternalSecretReadError::new("source_not_utf8"))?,
    );
    let value = text.trim_end_matches(['\r', '\n']);
    if value.is_empty() {
        return Err(ExternalSecretReadError::new("source_empty"));
    }
    if value.contains(['\r', '\n', '\0']) {
        return Err(ExternalSecretReadError::new("source_not_single_line"));
    }

    Ok(ExternalSecretValue {
        value: Zeroizing::new(value.to_string()),
        warning_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(path: &Path, secret_path: &Path) {
        std::fs::write(
            path,
            format!(
                "version = 1\n\n[sources.TEST_EXTERNAL_KEY]\ntype = \"file\"\npath = {:?}\n",
                secret_path.display().to_string()
            ),
        )
        .unwrap();
    }

    #[test]
    fn missing_config_is_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        let sources = ExternalSecretSources::load(&dir.path().join("missing.toml")).unwrap();

        assert_eq!(sources.configured_keys().count(), 0);
    }

    #[test]
    fn file_source_is_authoritative_and_rotates_live() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        let secret = dir.path().join("mounted-secret");
        std::fs::write(&secret, "first\n").unwrap();
        write_config(&config, &secret);
        let sources = ExternalSecretSources::load(&config).unwrap();

        assert_eq!(
            sources
                .resolve("TEST_EXTERNAL_KEY")
                .unwrap()
                .unwrap()
                .as_str(),
            "first"
        );
        std::fs::write(&secret, "rotated\n").unwrap();
        assert_eq!(
            sources
                .resolve("TEST_EXTERNAL_KEY")
                .unwrap()
                .unwrap()
                .as_str(),
            "rotated"
        );
        assert_eq!(
            sources.protected_paths().collect::<Vec<_>>(),
            vec![secret.as_path()]
        );
    }

    #[test]
    fn missing_source_is_typed_and_never_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        let missing = dir.path().join("missing-secret");
        write_config(&config, &missing);
        let sources = ExternalSecretSources::load(&config).unwrap();

        let error = sources.resolve("TEST_EXTERNAL_KEY").unwrap_err();
        assert_eq!(error.code(), "source_missing");
        assert_eq!(
            sources.statuses()[0].error_code.as_deref(),
            Some("source_missing")
        );
    }

    #[test]
    fn config_rejects_relative_paths_unknown_fields_and_future_versions() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        for raw in [
            "version = 1\n[sources.TEST_KEY]\ntype = \"file\"\npath = \"relative\"\n",
            "version = 1\nvalue = \"not-allowed\"\n",
            "version = 2\n",
        ] {
            std::fs::write(&config, raw).unwrap();
            let error = ExternalSecretSources::load(&config)
                .unwrap_err()
                .to_string();
            assert!(!error.contains("relative"), "{error}");
            assert!(!error.contains("not-allowed"), "{error}");
        }

        let oversized_key = "A".repeat(MAX_KEY_BYTES + 1);
        std::fs::write(
            &config,
            format!(
                "version = 1\n[sources.{oversized_key}]\ntype = \"file\"\npath = \"/tmp/secret\"\n"
            ),
        )
        .unwrap();
        let error = ExternalSecretSources::load(&config)
            .unwrap_err()
            .to_string();
        assert!(!error.contains(&oversized_key));
    }

    #[test]
    fn source_rejects_multiline_and_oversized_values() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        let secret = dir.path().join("mounted-secret");
        write_config(&config, &secret);
        let sources = ExternalSecretSources::load(&config).unwrap();

        std::fs::write(&secret, "line one\nline two\n").unwrap();
        assert_eq!(
            sources.resolve("TEST_EXTERNAL_KEY").unwrap_err().code(),
            "source_not_single_line"
        );
        std::fs::write(&secret, vec![b'x'; MAX_SECRET_BYTES as usize + 1]).unwrap();
        assert_eq!(
            sources.resolve("TEST_EXTERNAL_KEY").unwrap_err().code(),
            "source_too_large"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_rejects_group_writable_permissions_without_exposing_value() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        let secret = dir.path().join("mounted-secret");
        std::fs::write(&secret, "do-not-expose").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o620)).unwrap();
        write_config(&config, &secret);
        let sources = ExternalSecretSources::load(&config).unwrap();

        let status = &sources.statuses()[0];
        assert!(!status.ready);
        assert_eq!(
            status.error_code.as_deref(),
            Some("source_writable_by_others")
        );
        assert!(!serde_json::to_string(status)
            .unwrap()
            .contains("do-not-expose"));
    }

    #[cfg(unix)]
    #[test]
    fn registry_rejects_group_writable_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(SECRET_SOURCES_FILENAME);
        std::fs::write(&config, "version = 1\n").unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o620)).unwrap();

        let error = ExternalSecretSources::load(&config)
            .unwrap_err()
            .to_string();
        assert!(error.contains("must not be writable"));
    }
}
