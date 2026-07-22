use std::fs;
use std::path::{Component, Path};

use sha2::{Digest, Sha256};

use crate::workflow_learning_promotion_types::WorkflowPromotionError;

pub(super) fn ensure_exact_file(
    path: &Path,
    expected_sha256: &str,
) -> Result<(), WorkflowPromotionError> {
    let actual = optional_file_hash(path)?;
    if actual.as_deref() == Some(expected_sha256) {
        Ok(())
    } else {
        Err(WorkflowPromotionError::Conflict(format!(
            "target {} does not contain the expected revision",
            path.display()
        )))
    }
}

pub(super) fn optional_file_hash(path: &Path) -> Result<Option<String>, WorkflowPromotionError> {
    Ok(read_optional_regular(path)?.as_deref().map(sha256_hex))
}

pub(super) fn read_optional_regular(
    path: &Path,
) -> Result<Option<Vec<u8>>, WorkflowPromotionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(WorkflowPromotionError::UnsafeFilesystem(format!(
                "{} is not a regular file",
                path.display()
            )))
        }
        Ok(_) => Ok(Some(fs::read(path)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn read_required_regular(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} is not a regular file", path.display()),
        ));
    }
    fs::read(path)
}

pub(super) fn ensure_regular_file_or_absent(path: &Path) -> Result<(), WorkflowPromotionError> {
    read_optional_regular(path).map(|_| ())
}

pub(super) fn ensure_existing_directory_or_absent(
    path: &Path,
) -> Result<(), WorkflowPromotionError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(WorkflowPromotionError::UnsafeFilesystem(format!(
                "{} is not a real directory",
                path.display()
            )))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn ensure_descendant(root: &Path, target: &Path) -> Result<(), WorkflowPromotionError> {
    let relative = target.strip_prefix(root).map_err(|_| {
        WorkflowPromotionError::UnsafeFilesystem("path escapes Captain home".to_string())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(WorkflowPromotionError::UnsafeFilesystem(
                "path contains a non-normal component".to_string(),
            ));
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(WorkflowPromotionError::UnsafeFilesystem(format!(
                    "{} is a symlink",
                    current.display()
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(super) fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), WorkflowPromotionError> {
    match read_optional_regular(path)? {
        Some(existing) if existing == bytes => return Ok(()),
        Some(_) => {
            return Err(WorkflowPromotionError::Conflict(format!(
                "immutable file {} already contains different bytes",
                path.display()
            )))
        }
        None => {}
    }
    if captain_types::durable_fs::create_new(path, bytes)? {
        Ok(())
    } else if read_required_regular(path)? == bytes {
        Ok(())
    } else {
        Err(WorkflowPromotionError::Conflict(format!(
            "immutable file {} lost a creation race",
            path.display()
        )))
    }
}

pub(super) fn validate_identifier(
    label: &str,
    value: &str,
    max: usize,
) -> Result<(), WorkflowPromotionError> {
    let valid = !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(WorkflowPromotionError::InvalidRequest(format!(
            "{label} is not a safe identifier"
        )))
    }
}

pub(super) fn validate_hash(label: &str, value: &str) -> Result<(), WorkflowPromotionError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(WorkflowPromotionError::InvalidRequest(format!(
            "{label} must be a 64-character hex digest"
        )))
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(unix)]
pub(super) fn make_private_directory(path: &Path) -> Result<(), WorkflowPromotionError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn make_private_directory(_path: &Path) -> Result<(), WorkflowPromotionError> {
    Ok(())
}
