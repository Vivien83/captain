use super::NodeNativeServiceError;
use std::{fs, path::Path};

const MAX_SERVICE_DEFINITION_BYTES: u64 = 64 * 1024;

pub(super) fn ensure_runtime_directories(home: &Path) -> Result<(), NodeNativeServiceError> {
    let node = home.join("node");
    let logs = node.join("logs");
    captain_types::durable_fs::create_dir_all(&logs)
        .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)?;
    for directory in [&node, &logs] {
        let metadata = fs::symlink_metadata(directory)
            .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)?;
        if !metadata.file_type().is_dir() {
            return Err(NodeNativeServiceError::UnsafeDefinition);
        }
        restrict_directory_permissions(directory)?;
    }
    Ok(())
}

pub(super) fn install_definition(
    path: &Path,
    content: &[u8],
    force: bool,
) -> Result<Option<Vec<u8>>, NodeNativeServiceError> {
    let previous = read_safe_definition(path)?;
    if previous.as_deref() == Some(content) {
        return Ok(previous);
    }
    if previous.is_some() && !force {
        return Err(NodeNativeServiceError::AlreadyInstalled);
    }
    if previous.is_some() {
        captain_types::durable_fs::atomic_write(path, content)
            .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)?;
    } else if !captain_types::durable_fs::create_new(path, content)
        .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)?
    {
        return Err(NodeNativeServiceError::AlreadyInstalled);
    }
    restrict_definition_permissions(path)?;
    Ok(previous)
}

pub(super) fn restore_definition(
    path: &Path,
    previous: Option<&[u8]>,
) -> Result<(), NodeNativeServiceError> {
    match previous {
        Some(content) => {
            captain_types::durable_fs::atomic_write(path, content)
                .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)?;
            restrict_definition_permissions(path)
        }
        None => captain_types::durable_fs::remove_file(path)
            .map(|_| ())
            .map_err(|_| NodeNativeServiceError::DefinitionUnavailable),
    }
}

fn read_safe_definition(path: &Path) -> Result<Option<Vec<u8>>, NodeNativeServiceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if metadata.len() > MAX_SERVICE_DEFINITION_BYTES {
                return Err(NodeNativeServiceError::UnsafeDefinition);
            }
            fs::read(path)
                .map(Some)
                .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)
        }
        Ok(_) => Err(NodeNativeServiceError::UnsafeDefinition),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(NodeNativeServiceError::DefinitionUnavailable),
    }
}

pub(super) fn safe_definition_exists(path: &Path) -> Result<bool, NodeNativeServiceError> {
    read_safe_definition(path).map(|definition| definition.is_some())
}

pub(super) fn require_definition(path: &Path) -> Result<(), NodeNativeServiceError> {
    if safe_definition_exists(path)? {
        Ok(())
    } else {
        Err(NodeNativeServiceError::ActionFailed)
    }
}

pub(super) fn remove_definition(path: &Path) -> Result<(), NodeNativeServiceError> {
    require_definition(path)?;
    captain_types::durable_fs::remove_file(path)
        .map(|_| ())
        .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)
}

#[cfg(unix)]
fn restrict_definition_permissions(path: &Path) -> Result<(), NodeNativeServiceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)
}

#[cfg(not(unix))]
fn restrict_definition_permissions(_path: &Path) -> Result<(), NodeNativeServiceError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_directory_permissions(path: &Path) -> Result<(), NodeNativeServiceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| NodeNativeServiceError::DefinitionUnavailable)
}

#[cfg(not(unix))]
fn restrict_directory_permissions(_path: &Path) -> Result<(), NodeNativeServiceError> {
    Ok(())
}

pub(super) fn validate_runtime_path(
    path: &Path,
    require_file: bool,
) -> Result<(), NodeNativeServiceError> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return Err(NodeNativeServiceError::InvalidPath);
    }
    let metadata = fs::metadata(path).map_err(|_| NodeNativeServiceError::InvalidPath)?;
    if (require_file && !metadata.is_file()) || (!require_file && !metadata.is_dir()) {
        return Err(NodeNativeServiceError::InvalidPath);
    }
    Ok(())
}

pub(super) fn path_arg(path: &Path) -> Result<String, NodeNativeServiceError> {
    path.to_str()
        .filter(|value| !value.chars().any(char::is_control))
        .map(ToOwned::to_owned)
        .ok_or(NodeNativeServiceError::InvalidPath)
}
