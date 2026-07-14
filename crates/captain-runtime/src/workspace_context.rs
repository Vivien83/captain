//! Workspace context auto-detection.
//!
//! Scans the workspace root for project type indicators (Cargo.toml, package.json, etc.),
//! context files (custom AGENTS.md, CLAUDE.md, SOUL.md, TOOLS.md, IDENTITY.md,
//! HEARTBEAT.md), and Captain state files. Provides mtime-cached file reads to
//! avoid redundant I/O.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::debug;

/// Maximum file size to read for context files (32KB).
const MAX_FILE_SIZE: u64 = 32_768;

/// Known context file names scanned in the workspace root.
const CONTEXT_FILES: &[&str] = &[
    "AGENTS.override.md",
    "AGENTS.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    "CODEX.md",
    "CAPTAIN.md",
    "SOUL.md",
    "TOOLS.md",
    "IDENTITY.md",
    "HEARTBEAT.md",
];

/// Detected project type based on marker files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    Go,
    Java,
    DotNet,
    Unknown,
}

impl ProjectType {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Node => "Node.js",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::DotNet => ".NET",
            Self::Unknown => "Unknown",
        }
    }
}

/// Cached file content with modification time.
#[derive(Debug, Clone)]
struct CachedFile {
    content: String,
    mtime: SystemTime,
}

/// Workspace context information gathered from the project root.
#[derive(Debug)]
pub struct WorkspaceContext {
    /// The workspace root path.
    pub workspace_root: PathBuf,
    /// Detected project type.
    pub project_type: ProjectType,
    /// Whether this is a git repository.
    pub is_git_repo: bool,
    /// Whether .captain/ directory exists.
    pub has_captain_dir: bool,
    /// Cached context files.
    cache: HashMap<String, CachedFile>,
}

impl WorkspaceContext {
    /// Detect workspace context from the given root directory.
    pub fn detect(root: &Path) -> Self {
        let project_type = detect_project_type(root);
        let is_git_repo = root.join(".git").exists();
        let has_captain_dir = root.join(".captain").exists();

        let mut cache = HashMap::new();
        for &name in CONTEXT_FILES {
            let file_path = root.join(name);
            if let Some(cached) = read_cached_file(&file_path) {
                if workspace_context_file_has_product_content(name, &cached.content) {
                    debug!(file = name, "Loaded workspace context file");
                    cache.insert(name.to_string(), cached);
                }
            }
        }

        Self {
            workspace_root: root.to_path_buf(),
            project_type,
            is_git_repo,
            has_captain_dir,
            cache,
        }
    }

    /// Get the content of a cached context file, refreshing if mtime changed.
    pub fn get_file(&mut self, name: &str) -> Option<&str> {
        let file_path = self.workspace_root.join(name);

        // Check if we have a cached version
        if let Some(cached) = self.cache.get(name) {
            // Verify mtime hasn't changed
            if let Ok(meta) = std::fs::metadata(&file_path) {
                if let Ok(mtime) = meta.modified() {
                    if mtime == cached.mtime {
                        return self.cache.get(name).map(|c| c.content.as_str());
                    }
                }
            }
        }

        // Cache miss or mtime changed — re-read
        if let Some(new_cached) = read_cached_file(&file_path) {
            if workspace_context_file_has_product_content(name, &new_cached.content) {
                self.cache.insert(name.to_string(), new_cached);
                return self.cache.get(name).map(|c| c.content.as_str());
            }
        }

        // File doesn't exist or is too large
        self.cache.remove(name);
        None
    }

    /// Build a prompt context section summarizing the workspace.
    pub fn build_context_section(&mut self) -> String {
        let mut parts = Vec::new();

        parts.push(format!(
            "## Workspace Context\n- Project: {} ({})",
            self.workspace_root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".to_string()),
            self.project_type.label(),
        ));

        if self.is_git_repo {
            parts.push("- Git repository: yes".to_string());
        }

        // Include context file summaries
        let file_names: Vec<String> = self.cache.keys().cloned().collect();
        for name in file_names {
            if let Some(content) = self.get_file(&name) {
                let preview_cap = if is_guidance_context_file(&name) {
                    1000
                } else {
                    200
                };
                let preview = if content.len() > preview_cap {
                    format!(
                        "{}...",
                        crate::str_utils::safe_truncate_str(content, preview_cap)
                    )
                } else {
                    content.to_string()
                };
                parts.push(format!("### {}\n{}", name, preview));
            }
        }

        parts.join("\n")
    }
}

/// Read a file into the cache if it exists and is under the size limit.
fn read_cached_file(path: &Path) -> Option<CachedFile> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_SIZE {
        debug!(
            path = %path.display(),
            size = meta.len(),
            "Skipping oversized context file"
        );
        return None;
    }
    let mtime = meta.modified().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    Some(CachedFile { content, mtime })
}

fn workspace_context_file_has_product_content(name: &str, content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }

    match name {
        "TOOLS.md" => {
            !trimmed.contains("Agent-specific environment notes")
                && trimmed != "# Tools & Environment"
        }
        "AGENTS.md" | "AGENTS.override.md" => {
            let lower = trimmed.to_ascii_lowercase();
            let generated_agent_rules = lower.contains("# agent behavioral guidelines")
                && (lower.contains("## memory (mandatory)")
                    || lower.contains("## memory journal")
                    || lower.contains("update memory.md after significant actions"));
            !generated_agent_rules
        }
        _ => true,
    }
}

fn is_guidance_context_file(name: &str) -> bool {
    matches!(
        name,
        "AGENTS.override.md"
            | "AGENTS.md"
            | "CLAUDE.md"
            | "CLAUDE.local.md"
            | "CODEX.md"
            | "CAPTAIN.md"
    )
}

/// Detect project type from marker files in the root.
fn detect_project_type(root: &Path) -> ProjectType {
    if root.join("Cargo.toml").exists() {
        ProjectType::Rust
    } else if root.join("package.json").exists() {
        ProjectType::Node
    } else if root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("requirements.txt").exists()
    {
        ProjectType::Python
    } else if root.join("go.mod").exists() {
        ProjectType::Go
    } else if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        ProjectType::Java
    } else if root.join("*.csproj").exists() || root.join("*.sln").exists() {
        // Glob patterns don't work with exists(), so check differently
        if has_extension_in_dir(root, "csproj") || has_extension_in_dir(root, "sln") {
            ProjectType::DotNet
        } else {
            ProjectType::Unknown
        }
    } else {
        ProjectType::Unknown
    }
}

/// Check if any file with the given extension exists in a directory.
fn has_extension_in_dir(dir: &Path, ext: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(e) = entry.path().extension() {
                if e == ext {
                    return true;
                }
            }
        }
    }
    false
}

/// Persistent workspace state, saved to `.captain/workspace-state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// State format version.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Timestamp when bootstrap was first seeded.
    pub bootstrap_seeded_at: Option<String>,
    /// Timestamp when onboarding was completed.
    pub onboarding_completed_at: Option<String>,
}

fn default_version() -> u32 {
    1
}

impl WorkspaceState {
    /// Load state from the workspace's `.captain/workspace-state.json`.
    pub fn load(workspace_root: &Path) -> Self {
        let path = workspace_root.join(".captain").join("workspace-state.json");
        match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save state to the workspace's `.captain/workspace-state.json`.
    pub fn save(&self, workspace_root: &Path) -> Result<(), String> {
        let dir = workspace_root.join(".captain");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create .captain dir: {e}"))?;
        let path = dir.join("workspace-state.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize state: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("Failed to write state: {e}"))
    }
}

#[cfg(test)]
#[path = "workspace_context_tests.rs"]
mod tests;
