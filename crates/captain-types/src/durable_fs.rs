//! Crash-safe filesystem primitives for Captain's operational state.
//!
//! A successful write means the new file contents and directory entry have
//! both been synchronized. The temporary file always lives beside the target,
//! so activation is an atomic replacement on supported platforms.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// Create a directory tree and synchronize every new directory entry.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    ensure_directory(path)
}

/// Atomically replace `path` with `contents` and synchronize the result.
///
/// Newly created files are private on Unix (`0600`, inherited from
/// `NamedTempFile`). Callers should use a different primitive for public or
/// executable artifacts.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = normalized_parent(path);
    ensure_directory(parent)?;

    let mut pending = tempfile::Builder::new()
        .prefix(".captain-write-")
        .tempfile_in(parent)?;
    pending.write_all(contents)?;
    commit_pending(pending, path, parent)
}

/// Copy a file through the same durable atomic replacement protocol.
pub fn atomic_copy(source: &Path, destination: &Path) -> io::Result<u64> {
    let parent = normalized_parent(destination);
    ensure_directory(parent)?;

    let mut source = File::open(source)?;
    let mut pending = tempfile::Builder::new()
        .prefix(".captain-copy-")
        .tempfile_in(parent)?;
    let copied = io::copy(&mut source, &mut pending)?;
    commit_pending(pending, destination, parent)?;
    Ok(copied)
}

/// Durably create a new file without ever replacing an existing one.
///
/// Returns `false` when `path` already exists. The contents are staged and
/// synchronized before the name becomes visible, so a crash cannot leave a
/// partially initialized file that future boots mistake for user-owned data.
pub fn create_new(path: &Path, contents: &[u8]) -> io::Result<bool> {
    let parent = normalized_parent(path);
    ensure_directory(parent)?;

    let mut pending = tempfile::Builder::new()
        .prefix(".captain-create-")
        .tempfile_in(parent)?;
    pending.write_all(contents)?;
    pending.flush()?;
    sync_file(pending.as_file())?;

    match pending.persist_noclobber(path) {
        Ok(_) => {
            sync_directory(parent)?;
            Ok(true)
        }
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error.error),
    }
}

fn commit_pending(
    mut pending: tempfile::NamedTempFile,
    path: &Path,
    parent: &Path,
) -> io::Result<()> {
    pending.flush()?;
    sync_file(pending.as_file())?;

    pending.persist(path).map_err(|error| error.error)?;
    sync_directory(parent)
}

/// Remove a state file and synchronize its containing directory.
///
/// Returns `false` when the file was already absent.
pub fn remove_file(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            sync_directory(normalized_parent(path))?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn ensure_directory(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }

    let mut missing = Vec::new();
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.as_os_str().is_empty() {
            break;
        }
        if candidate.is_dir() {
            break;
        }
        missing.push(candidate);
        current = candidate.parent();
    }

    std::fs::create_dir_all(path)?;
    for created in missing.into_iter().rev() {
        sync_directory(normalized_parent(created))?;
    }
    Ok(())
}

fn sync_file(file: &File) -> io::Result<()> {
    file.sync_all()?;
    full_sync_file(file)
}

#[cfg(target_os = "macos")]
fn full_sync_file(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    // `fsync` may return before the drive has flushed its volatile cache on
    // macOS. F_FULLFSYNC is the platform primitive for committed state that
    // must survive sudden power loss.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn full_sync_file(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces_complete_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/state.json");

        atomic_write(&path, br#"{"generation":1}"#).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"generation":1}"#);

        atomic_write(&path, br#"{"generation":2,"complete":true}"#).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            br#"{"generation":2,"complete":true}"#
        );
        assert!(std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".captain-write-")));
    }

    #[test]
    fn concurrent_writers_never_leave_a_torn_payload() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.json");
        let payloads = (0..12)
            .map(|index| format!(r#"{{"writer":{index},"body":"{}"}}"#, "x".repeat(4096)))
            .collect::<Vec<_>>();
        let handles = payloads
            .iter()
            .cloned()
            .map(|payload| {
                let path = path.clone();
                std::thread::spawn(move || atomic_write(&path, payload.as_bytes()))
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().unwrap().unwrap();
        }

        let actual = std::fs::read_to_string(&path).unwrap();
        assert!(payloads.contains(&actual));
    }

    #[test]
    fn remove_file_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.json");
        atomic_write(&path, b"state").unwrap();

        assert!(remove_file(&path).unwrap());
        assert!(!remove_file(&path).unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn atomic_copy_replaces_the_destination() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.toml");
        let destination = root.path().join("nested/config.toml");
        std::fs::write(&source, b"version = 2\n").unwrap();
        atomic_write(&destination, b"version = 1\n").unwrap();

        assert_eq!(atomic_copy(&source, &destination).unwrap(), 12);
        assert_eq!(std::fs::read(destination).unwrap(), b"version = 2\n");
    }

    #[test]
    fn create_new_never_overwrites_existing_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested/IDENTITY.md");

        assert!(create_new(&path, b"generated").unwrap());
        assert!(!create_new(&path, b"replacement").unwrap());
        assert_eq!(std::fs::read(path).unwrap(), b"generated");
    }

    #[cfg(unix)]
    #[test]
    fn newly_created_state_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("secret.json");
        atomic_write(&path, b"secret").unwrap();

        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
