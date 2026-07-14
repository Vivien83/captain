//! Linked-file discovery and path guards for file-backed skills.

use std::path::{Component, Path};

pub(crate) fn path_has_traversal(file_path: &str) -> bool {
    let path = Path::new(file_path);
    path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

pub(crate) fn linked_files(skill_root: &Path) -> serde_json::Map<String, serde_json::Value> {
    let mut references = Vec::new();
    let mut templates = Vec::new();
    let mut scripts = Vec::new();
    let mut assets = Vec::new();
    let mut other = Vec::new();

    collect_linked_files(
        skill_root,
        skill_root,
        0,
        &mut references,
        &mut templates,
        &mut scripts,
        &mut assets,
        &mut other,
    );

    let mut out = serde_json::Map::new();
    push_files(&mut out, "references", references);
    push_files(&mut out, "templates", templates);
    push_files(&mut out, "scripts", scripts);
    push_files(&mut out, "assets", assets);
    push_files(&mut out, "other", other);
    out
}

#[allow(clippy::too_many_arguments)]
fn collect_linked_files(
    root: &Path,
    current: &Path,
    depth: usize,
    references: &mut Vec<String>,
    templates: &mut Vec<String>,
    scripts: &mut Vec<String>,
    assets: &mut Vec<String>,
    other: &mut Vec<String>,
) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if file_name == ".git" || file_name == ".github" || file_name == ".hub" {
            continue;
        }
        if path.is_dir() {
            collect_linked_files(
                root,
                &path,
                depth + 1,
                references,
                templates,
                scripts,
                assets,
                other,
            );
            continue;
        }
        if !path.is_file() || file_name == "SKILL.md" || file_name == "skill.toml" {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().to_string();
        if rel.starts_with("references/") {
            references.push(rel);
        } else if rel.starts_with("templates/") {
            templates.push(rel);
        } else if rel.starts_with("scripts/") {
            scripts.push(rel);
        } else if rel.starts_with("assets/") {
            assets.push(rel);
        } else if is_supported_skill_file(&path) {
            other.push(rel);
        }
    }
}

fn push_files(
    out: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    mut files: Vec<String>,
) {
    if files.is_empty() {
        return;
    }
    files.sort();
    out.insert(key.to_string(), serde_json::json!(files));
}

fn is_supported_skill_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("md" | "txt" | "json" | "toml" | "yaml" | "yml" | "py" | "js" | "ts" | "sh")
    )
}
