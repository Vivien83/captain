//! Persistent PTY-backed shell session for the Computer Panel (v3.9a).
//!
//! Wraps `portable-pty` to expose a long-running shell whose cwd/env persists
//! across multiple writes — the foundation for the Manus-like live terminal
//! in the Next.js Computer Panel (v3.9d). Inputs arrive via `write_stdin`;
//! output is streamed through an mpsc channel of `PtyEvent`s.
//!
//! The actor owns the PTY child; dropping it terminates the process tree.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tokio::sync::mpsc;
use tokio::task;
use tracing::{debug, warn};

/// Events emitted by a [`SessionActor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    /// Raw bytes read from the child's stdout/stderr (merged).
    Output(Vec<u8>),
    /// Child exited with the given status code (None if signaled).
    Exited(Option<i32>),
    /// An unrecoverable I/O error on the reader thread.
    Error(String),
}

/// Spawn configuration.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// Shell binary (e.g. "bash", "zsh", "sh"). Empty = pick the user's default.
    pub shell: Option<String>,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Initial working directory.
    pub cwd: Option<String>,
    /// Extra environment variables (merged on top of inherited env).
    pub env: Vec<(String, String)>,
    /// Environment variables to remove from the inherited environment.
    pub remove_env: Vec<String>,
    /// Initial terminal size.
    pub rows: u16,
    pub cols: u16,
}

impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            shell: None,
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            remove_env: Vec::new(),
            rows: 24,
            cols: 80,
        }
    }
}

/// Handle to a running PTY session.
///
/// `write_stdin` feeds the child. Drop the `SessionActor` to terminate.
pub struct SessionActor {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl SessionActor {
    /// Spawn a PTY-backed shell. Output flows into `tx`.
    pub fn spawn(spec: SessionSpec, tx: mpsc::Sender<PtyEvent>) -> Result<Self, String> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: spec.rows,
                cols: spec.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("openpty failed: {e}"))?;

        let shell = spec.shell.unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&shell);
        cmd.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            cmd.cwd(cwd);
        }
        for key in &spec.remove_env {
            cmd.env_remove(key);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("spawn_command failed: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("take_writer failed: {e}"))?;
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("try_clone_reader failed: {e}"))?;

        let tx_reader = tx.clone();
        task::spawn_blocking(move || reader_loop(reader, tx_reader));

        let child = Arc::new(Mutex::new(child));
        let child_waiter = Arc::clone(&child);
        let tx_exit = tx.clone();
        task::spawn_blocking(move || loop {
            let status = {
                let mut guard = match child_waiter.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                match guard.try_wait() {
                    Ok(Some(status)) => Some(status),
                    Ok(None) => None,
                    Err(e) => {
                        let _ = tx_exit.blocking_send(PtyEvent::Error(format!("wait error: {e}")));
                        return;
                    }
                }
            };
            if let Some(status) = status {
                let code = status.exit_code() as i32;
                let _ = tx_exit.blocking_send(PtyEvent::Exited(Some(code)));
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        });

        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            master: Arc::new(Mutex::new(pair.master)),
            child,
        })
    }

    /// Feed bytes to the child's stdin (e.g. `"ls\n"`).
    pub fn write_stdin(&self, data: &[u8]) -> Result<(), String> {
        let mut w = self
            .writer
            .lock()
            .map_err(|e| format!("writer lock poisoned: {e}"))?;
        w.write_all(data).map_err(|e| format!("write_stdin: {e}"))?;
        w.flush().map_err(|e| format!("flush: {e}"))?;
        Ok(())
    }

    /// Resize the PTY (xterm.js addon-fit sends this on layout changes).
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let master = self
            .master
            .lock()
            .map_err(|e| format!("master lock poisoned: {e}"))?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("resize: {e}"))
    }

    /// Kill the PTY child. Non-blocking; drop the actor to free the master.
    pub fn terminate(&self) -> Result<(), String> {
        let mut child = self
            .child
            .lock()
            .map_err(|e| format!("child lock poisoned: {e}"))?;
        child.kill().map_err(|e| format!("kill: {e}"))
    }
}

impl Drop for SessionActor {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
        }
    }
}

fn reader_loop(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<PtyEvent>) {
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if tx
                    .blocking_send(PtyEvent::Output(buf[..n].to_vec()))
                    .is_err()
                {
                    debug!("pty_session: receiver dropped, stopping reader");
                    return;
                }
            }
            Err(e) => {
                warn!("pty_session reader error: {e}");
                let _ = tx.blocking_send(PtyEvent::Error(format!("read: {e}")));
                return;
            }
        }
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// v3.9a — spawn a shell, send `echo hora\n`, receive the output.
    #[tokio::test]
    async fn spawn_write_read_echo() {
        let (tx, mut rx) = mpsc::channel::<PtyEvent>(32);
        let actor = SessionActor::spawn(
            SessionSpec {
                shell: Some("/bin/sh".to_string()),
                ..SessionSpec::default()
            },
            tx,
        )
        .expect("spawn should succeed");

        actor
            .write_stdin(b"echo hora-9a\n")
            .expect("write_stdin ok");

        let mut accumulated = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(PtyEvent::Output(bytes))) => {
                    accumulated.extend_from_slice(&bytes);
                    let text = String::from_utf8_lossy(&accumulated);
                    if text.contains("hora-9a") {
                        return;
                    }
                }
                Ok(Some(PtyEvent::Exited(_))) => break,
                Ok(Some(PtyEvent::Error(e))) => panic!("reader error: {e}"),
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        panic!(
            "expected 'hora-9a' in PTY output within 3s, got: {:?}",
            String::from_utf8_lossy(&accumulated)
        );
    }

    /// v3.9a — cwd persists between writes: `cd /tmp` then `pwd` shows /tmp.
    #[tokio::test]
    async fn cwd_persists_across_writes() {
        let (tx, mut rx) = mpsc::channel::<PtyEvent>(64);
        let actor = SessionActor::spawn(
            SessionSpec {
                shell: Some("/bin/sh".to_string()),
                ..SessionSpec::default()
            },
            tx,
        )
        .expect("spawn ok");

        actor.write_stdin(b"cd /tmp\n").unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        actor.write_stdin(b"pwd\n").unwrap();

        let mut accumulated = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(PtyEvent::Output(bytes))) => {
                    accumulated.extend_from_slice(&bytes);
                    let text = String::from_utf8_lossy(&accumulated);
                    if text.lines().any(|line| line.trim() == "/tmp") {
                        return;
                    }
                }
                Ok(Some(PtyEvent::Exited(_))) => break,
                Ok(Some(PtyEvent::Error(e))) => panic!("reader error: {e}"),
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        panic!(
            "expected '/tmp' as pwd output, got: {:?}",
            String::from_utf8_lossy(&accumulated)
        );
    }

    #[tokio::test]
    async fn remove_env_hides_inherited_variables() {
        let key = "CAPTAIN_PTY_REMOVE_ENV_TEST";
        std::env::set_var(key, "present");

        let (tx, mut rx) = mpsc::channel::<PtyEvent>(32);
        let actor = SessionActor::spawn(
            SessionSpec {
                shell: Some("/bin/sh".to_string()),
                args: vec![
                    "-lc".to_string(),
                    format!("printf '%s\\n' \"${{{key}:-missing}}\""),
                ],
                remove_env: vec![key.to_string()],
                ..SessionSpec::default()
            },
            tx,
        )
        .expect("spawn should succeed");
        std::env::remove_var(key);

        let mut accumulated = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(PtyEvent::Output(bytes))) => {
                    accumulated.extend_from_slice(&bytes);
                    let text = String::from_utf8_lossy(&accumulated);
                    assert!(
                        !text.contains("present"),
                        "removed env var leaked into child output: {text:?}"
                    );
                    if text.contains("missing") {
                        drop(actor);
                        return;
                    }
                }
                Ok(Some(PtyEvent::Exited(_))) => break,
                Ok(Some(PtyEvent::Error(e))) => panic!("reader error: {e}"),
                Ok(None) => break,
                Err(_) => continue,
            }
        }
        panic!(
            "expected removed env var to be missing, got: {:?}",
            String::from_utf8_lossy(&accumulated)
        );
    }

    /// v3.9a — drop terminates the child (no orphan PTYs).
    #[tokio::test]
    async fn drop_terminates_child() {
        let (tx, mut rx) = mpsc::channel::<PtyEvent>(16);
        {
            let actor = SessionActor::spawn(
                SessionSpec {
                    shell: Some("/bin/sh".to_string()),
                    ..SessionSpec::default()
                },
                tx,
            )
            .expect("spawn ok");
            actor.write_stdin(b"sleep 30\n").unwrap();
            // actor is dropped here
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline {
            if let Ok(Some(PtyEvent::Exited(_))) =
                tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
            {
                return;
            }
        }
        panic!("dropped SessionActor should terminate the child within 2s");
    }
}
