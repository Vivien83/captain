//! Interactive process manager — persistent process sessions.
//!
//! Allows agents to start long-running processes (REPLs, servers, watchers),
//! write to their stdin, read from stdout/stderr, and kill them.

use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::process_registry::{
    now_unix_secs, pid_is_alive, ProcessRegistryRecord, ProcessRegistryStore, RecoveredProcess,
};

pub type ProcessId = String;

struct ManagedProcess {
    stdin: Option<tokio::process::ChildStdin>,
    stdout_buf: Arc<Mutex<Vec<String>>>,
    stderr_buf: Arc<Mutex<Vec<String>>>,
    child: tokio::process::Child,
    pid: Option<u32>,
    agent_id: String,
    command: String,
    started_at: Instant,
    started_at_unix_secs: u64,
    last_activity_at: Arc<StdMutex<Instant>>,
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub id: ProcessId,
    pub agent_id: String,
    pub command: String,
    pub alive: bool,
    pub attached: bool,
    pub pid: Option<u32>,
    pub uptime_secs: u64,
    pub idle_secs: u64,
}

pub struct ProcessManager {
    processes: DashMap<ProcessId, ManagedProcess>,
    recovered: DashMap<ProcessId, RecoveredProcess>,
    max_per_agent: usize,
    next_id: std::sync::atomic::AtomicU64,
    registry: Option<ProcessRegistryStore>,
}

impl ProcessManager {
    pub fn new(max_per_agent: usize) -> Self {
        Self::with_registry(max_per_agent, None)
    }

    pub fn with_registry_path(max_per_agent: usize, path: impl Into<PathBuf>) -> Self {
        Self::with_registry(max_per_agent, Some(ProcessRegistryStore::new(path.into())))
    }

    fn with_registry(max_per_agent: usize, registry: Option<ProcessRegistryStore>) -> Self {
        let mut next_id = 1;
        let recovered = DashMap::new();

        if let Some(store) = &registry {
            let records = store.load_records();
            let mut live_records = Vec::new();
            for record in records {
                next_id = next_id.max(process_sequence(&record.id).unwrap_or(0) + 1);
                let Some(pid) = record.pid else {
                    continue;
                };
                if !pid_is_alive(pid) {
                    continue;
                }
                live_records.push(record.clone());
                recovered.insert(record.id.clone(), RecoveredProcess::from(record));
            }
            if let Err(error) = store.save_records(&live_records) {
                warn!(%error, "Could not rewrite recovered process registry");
            }
        }

        Self {
            processes: DashMap::new(),
            recovered,
            max_per_agent,
            next_id: std::sync::atomic::AtomicU64::new(next_id),
            registry,
        }
    }

    pub async fn start(
        &self,
        agent_id: &str,
        command: &str,
        args: &[String],
    ) -> Result<ProcessId, String> {
        self.start_in_dir(agent_id, command, args, None).await
    }

    pub async fn start_in_dir(
        &self,
        agent_id: &str,
        command: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Result<ProcessId, String> {
        self.ensure_agent_process_capacity(agent_id)?;
        let mut child = spawn_process_child(command, args, cwd)?;
        let pid = child.id();

        let stdin = child.stdin.take();
        let stdout_buf = Arc::new(Mutex::new(Vec::<String>::new()));
        let stderr_buf = Arc::new(Mutex::new(Vec::<String>::new()));
        let last_activity_at = Arc::new(StdMutex::new(Instant::now()));
        let started_at_unix_secs = now_unix_secs();

        spawn_process_output_reader(
            child.stdout.take(),
            stdout_buf.clone(),
            last_activity_at.clone(),
        );
        spawn_process_output_reader(
            child.stderr.take(),
            stderr_buf.clone(),
            last_activity_at.clone(),
        );

        let id = self.next_process_id();
        let cmd_display = process_command_display(command, args);

        debug!(process_id = %id, command = %cmd_display, agent = %agent_id, "Started persistent process");

        self.processes.insert(
            id.clone(),
            ManagedProcess {
                stdin,
                stdout_buf,
                stderr_buf,
                child,
                pid,
                agent_id: agent_id.to_string(),
                command: cmd_display,
                started_at: Instant::now(),
                started_at_unix_secs,
                last_activity_at,
            },
        );
        self.persist_registry_best_effort();

        Ok(id)
    }

    fn ensure_agent_process_capacity(&self, agent_id: &str) -> Result<(), String> {
        let agent_count = self.agent_process_count(agent_id);
        if agent_count >= self.max_per_agent {
            return Err(format!(
                "Agent '{}' already has {} processes (max: {}). Call process_list \
                 to see what's running and process_kill to free a slot before \
                 starting another. If you don't actually need a long-lived \
                 process (a server, watcher, REPL), use tool_run_start instead \
                 for a one-shot detached command — it isn't subject to this cap.",
                agent_id, agent_count, self.max_per_agent
            ));
        }
        Ok(())
    }

    fn agent_process_count(&self, agent_id: &str) -> usize {
        self.processes
            .iter()
            .filter(|entry| entry.value().agent_id == agent_id)
            .count()
            + self
                .recovered
                .iter()
                .filter(|entry| {
                    entry.value().agent_id == agent_id && pid_is_alive(entry.value().pid)
                })
                .count()
    }

    fn next_process_id(&self) -> ProcessId {
        format!(
            "proc_{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        )
    }

    pub async fn write(&self, process_id: &str, data: &str) -> Result<(), String> {
        if self.recovered.contains_key(process_id) {
            return Err(format!(
                "Process '{process_id}' was recovered after restart; stdin is not attached. Stop it with `captain process kill {process_id}` and start a new process if needed."
            ));
        }

        let mut entry = self
            .processes
            .get_mut(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

        let proc = entry.value_mut();
        if let Some(stdin) = &mut proc.stdin {
            stdin
                .write_all(data.as_bytes())
                .await
                .map_err(|e| format!("Write failed: {}", e))?;
            stdin
                .flush()
                .await
                .map_err(|e| format!("Flush failed: {}", e))?;
            mark_activity(&proc.last_activity_at);
            drop(entry);
            self.persist_registry_best_effort();
            Ok(())
        } else {
            Err("Process stdin is closed".to_string())
        }
    }

    pub async fn read(&self, process_id: &str) -> Result<(Vec<String>, Vec<String>), String> {
        if self.recovered.contains_key(process_id) {
            return Err(format!(
                "Process '{process_id}' was recovered after restart; stdout/stderr are not attached. Stop it with `captain process kill {process_id}` or inspect it externally."
            ));
        }

        let entry = self
            .processes
            .get(process_id)
            .ok_or_else(|| format!("Process '{}' not found", process_id))?;

        let mut stdout = entry.stdout_buf.lock().await;
        let mut stderr = entry.stderr_buf.lock().await;

        let out_lines: Vec<String> = stdout.drain(..).collect();
        let err_lines: Vec<String> = stderr.drain(..).collect();

        Ok((out_lines, err_lines))
    }

    pub async fn kill(&self, process_id: &str) -> Result<(), String> {
        if let Some((_, mut proc)) = self.processes.remove(process_id) {
            if let Some(pid) = proc.child.id().or(proc.pid) {
                debug!(process_id, pid, "Killing persistent process");
                let _ = crate::subprocess_sandbox::kill_process_tree(pid, 3000).await;
            }
            let _ = proc.child.kill().await;
            self.persist_registry_best_effort();
            return Ok(());
        }

        if let Some((_, proc)) = self.recovered.remove(process_id) {
            let pid = proc.pid;
            debug!(process_id, pid, "Killing persistent process");
            let _ = crate::subprocess_sandbox::kill_process_tree(pid, 3000).await;
            self.persist_registry_best_effort();
            return Ok(());
        }

        Err(format!("Process '{}' not found", process_id))
    }

    pub fn list(&self, agent_id: &str) -> Vec<ProcessInfo> {
        let mut processes: Vec<ProcessInfo> = self
            .processes
            .iter_mut()
            .filter_map(|mut entry| {
                if entry.value().agent_id != agent_id {
                    return None;
                }
                let id = entry.key().clone();
                Some(process_info(&id, entry.value_mut()))
            })
            .collect();
        processes.extend(self.recovered.iter().filter_map(|entry| {
            if entry.value().agent_id != agent_id {
                return None;
            }
            Some(recovered_process_info(entry.key(), entry.value()))
        }));
        processes
    }

    pub fn list_all(&self) -> Vec<ProcessInfo> {
        let mut processes: Vec<ProcessInfo> = self
            .processes
            .iter_mut()
            .map(|mut entry| {
                let id = entry.key().clone();
                process_info(&id, entry.value_mut())
            })
            .collect();
        processes.extend(
            self.recovered
                .iter()
                .map(|entry| recovered_process_info(entry.key(), entry.value())),
        );
        processes
    }

    pub async fn cleanup(&self, max_age_secs: u64) {
        let to_remove: Vec<ProcessId> = self
            .processes
            .iter_mut()
            .filter_map(|mut entry| {
                if entry.value().started_at.elapsed().as_secs() <= max_age_secs {
                    return None;
                }
                match entry.value_mut().child.try_wait() {
                    Ok(Some(status)) => {
                        debug!(
                            process_id = %entry.key(),
                            %status,
                            "Reaping old exited persistent process"
                        );
                        Some(entry.key().clone())
                    }
                    Ok(None) => {
                        warn!(
                            process_id = %entry.key(),
                            idle_secs = entry.value().last_activity_at_idle_secs(),
                            "Persistent process is old but still live; cleanup leaves it running"
                        );
                        None
                    }
                    Err(e) => {
                        warn!(
                            process_id = %entry.key(),
                            error = %e,
                            "Could not inspect persistent process state; removing stale handle"
                        );
                        Some(entry.key().clone())
                    }
                }
            })
            .collect();

        for id in to_remove {
            let _ = self.processes.remove(&id);
        }

        let recovered_to_remove: Vec<ProcessId> = self
            .recovered
            .iter()
            .filter_map(|entry| {
                if !pid_is_alive(entry.value().pid) {
                    return Some(entry.key().clone());
                }
                let age = now_unix_secs().saturating_sub(entry.value().started_at_unix_secs);
                if age > max_age_secs {
                    warn!(
                        process_id = %entry.key(),
                        pid = entry.value().pid,
                        "Recovered persistent process is old but still live; cleanup leaves it running"
                    );
                }
                None
            })
            .collect();

        for id in recovered_to_remove {
            let _ = self.recovered.remove(&id);
        }

        self.persist_registry_best_effort();
    }

    pub fn count(&self) -> usize {
        self.processes.len() + self.recovered.len()
    }

    fn persist_registry_best_effort(&self) {
        let Some(registry) = &self.registry else {
            return;
        };

        let mut records = Vec::new();
        for mut entry in self.processes.iter_mut() {
            let id = entry.key().clone();
            let process = entry.value_mut();
            if !matches!(process.child.try_wait(), Ok(None)) {
                continue;
            }
            records.push(ProcessRegistryRecord {
                id,
                agent_id: process.agent_id.clone(),
                command: process.command.clone(),
                pid: process.pid,
                started_at_unix_secs: process.started_at_unix_secs,
                last_activity_unix_secs: process.last_activity_unix_secs(),
            });
        }

        for entry in self.recovered.iter() {
            if !pid_is_alive(entry.value().pid) {
                continue;
            }
            records.push(ProcessRegistryRecord {
                id: entry.key().clone(),
                agent_id: entry.value().agent_id.clone(),
                command: entry.value().command.clone(),
                pid: Some(entry.value().pid),
                started_at_unix_secs: entry.value().started_at_unix_secs,
                last_activity_unix_secs: entry.value().last_activity_unix_secs,
            });
        }

        records.sort_by(|a, b| a.id.cmp(&b.id));
        if let Err(error) = registry.save_records(&records) {
            warn!(%error, "Could not persist process registry");
        }
    }
}

fn spawn_process_child(
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<tokio::process::Child, String> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    crate::env_sandbox::apply_minimal_env(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("Failed to start process '{}': {}", command, e))
}

fn spawn_process_output_reader<R>(
    stream: Option<R>,
    buffer: Arc<Mutex<Vec<String>>>,
    activity: Arc<StdMutex<Instant>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(stream) = stream else {
        return;
    };

    tokio::spawn(async move {
        let reader = BufReader::new(stream);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            mark_activity(&activity);
            let mut buffer = buffer.lock().await;
            push_process_output_line(&mut buffer, line);
        }
    });
}

fn push_process_output_line(buffer: &mut Vec<String>, line: String) {
    if buffer.len() >= 1000 {
        buffer.drain(..100);
    }
    buffer.push(line);
}

fn process_command_display(command: &str, args: &[String]) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    }
}

fn process_info(id: &str, process: &mut ManagedProcess) -> ProcessInfo {
    let alive = matches!(process.child.try_wait(), Ok(None));
    ProcessInfo {
        id: id.to_string(),
        agent_id: process.agent_id.clone(),
        command: process.command.clone(),
        alive,
        attached: true,
        pid: process.pid,
        uptime_secs: process.started_at.elapsed().as_secs(),
        idle_secs: process.last_activity_at_idle_secs(),
    }
}

fn recovered_process_info(id: &str, process: &RecoveredProcess) -> ProcessInfo {
    let now = now_unix_secs();
    ProcessInfo {
        id: id.to_string(),
        agent_id: process.agent_id.clone(),
        command: process.command.clone(),
        alive: pid_is_alive(process.pid),
        attached: false,
        pid: Some(process.pid),
        uptime_secs: now.saturating_sub(process.started_at_unix_secs),
        idle_secs: now.saturating_sub(process.last_activity_unix_secs),
    }
}

impl ManagedProcess {
    fn last_activity_at_idle_secs(&self) -> u64 {
        self.last_activity_at
            .lock()
            .map(|instant| instant.elapsed().as_secs())
            .unwrap_or(0)
    }

    fn last_activity_unix_secs(&self) -> u64 {
        now_unix_secs().saturating_sub(self.last_activity_at_idle_secs())
    }
}

fn mark_activity(activity: &Arc<StdMutex<Instant>>) {
    if let Ok(mut last_seen) = activity.lock() {
        *last_seen = Instant::now();
    }
}

fn process_sequence(id: &str) -> Option<u64> {
    id.strip_prefix("proc_")?.parse().ok()
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new(5)
    }
}

#[cfg(test)]
#[path = "process_manager_tests.rs"]
mod process_manager_tests;
