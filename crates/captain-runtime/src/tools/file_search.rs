//! Gitignore-aware file search and batched inspection handlers.

use crate::kernel_handle::KernelHandle;
use crate::tools::{resolve_file_path_for_caller, tool_file_list, tool_file_read, truncate_owned};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

const MAX_FILE_INSPECT_BATCH_ITEMS: usize = 30;

struct GrepRequest<'a> {
    pattern: &'a str,
    case_insensitive: bool,
    multiline: bool,
    before: usize,
    after: usize,
    head_limit: usize,
    output_mode: &'a str,
    raw_path: &'a str,
    glob_pattern: Option<&'a str>,
    file_type: Option<&'a str>,
}

#[derive(Default)]
struct GrepResults {
    lines: Vec<String>,
    total_matches: usize,
    files_with_matches: usize,
    hit_cap: bool,
}

struct GlobRequest<'a> {
    pattern: &'a str,
    raw_path: &'a str,
    head_limit: usize,
}

struct FileInspectBatchRequest<'a> {
    operations: &'a [serde_json::Value],
    max_read_chars: usize,
    stop_on_error: bool,
}

pub(crate) async fn tool_grep(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let request = parse_grep_request(input)?;
    let root =
        resolve_file_path_for_caller(request.raw_path, workspace_root, kernel, caller_agent_id)?;
    let re = compile_grep_regex(&request)?;
    let glob_matcher = compile_grep_glob(request.glob_pattern)?;
    let type_exts = request.file_type.map(grep_type_extensions);
    let mut results = GrepResults::default();

    'walk: for entry in file_search_walk_builder(&root).build().flatten() {
        let Some((rel, lines, matched_line_indices)) =
            grep_entry_matches(&entry, &root, &re, glob_matcher.as_ref(), type_exts)
        else {
            continue;
        };
        results.total_matches += matched_line_indices.len();
        results.files_with_matches += 1;
        emit_grep_result(
            request.output_mode,
            &mut results.lines,
            &rel,
            &lines,
            &matched_line_indices,
            request.before,
            request.after,
        )?;

        if results.lines.len() >= request.head_limit {
            results.hit_cap = true;
            break 'walk;
        }
    }

    Ok(format_grep_results(&request, &root, results))
}

fn parse_grep_request(input: &serde_json::Value) -> Result<GrepRequest<'_>, String> {
    let pattern = input["pattern"]
        .as_str()
        .ok_or_else(|| "Missing 'pattern' parameter".to_string())?;
    let context = input["-C"].as_u64().unwrap_or(0) as usize;
    let after = input["-A"].as_u64().unwrap_or(0) as usize;
    let before = input["-B"].as_u64().unwrap_or(0) as usize;

    Ok(GrepRequest {
        pattern,
        case_insensitive: input["-i"].as_bool().unwrap_or(false),
        multiline: input["multiline"].as_bool().unwrap_or(false),
        after: if context > 0 { context } else { after },
        before: if context > 0 { context } else { before },
        head_limit: input["head_limit"].as_u64().unwrap_or(250) as usize,
        output_mode: input["output_mode"]
            .as_str()
            .unwrap_or("files_with_matches"),
        raw_path: input["path"].as_str().unwrap_or("."),
        glob_pattern: input["glob"].as_str(),
        file_type: input["type"].as_str(),
    })
}

fn compile_grep_regex(request: &GrepRequest<'_>) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(request.pattern)
        .case_insensitive(request.case_insensitive)
        .multi_line(true)
        .dot_matches_new_line(request.multiline)
        .build()
        .map_err(|e| format!("Invalid regex: {e}"))
}

fn compile_grep_glob(glob_pattern: Option<&str>) -> Result<Option<globset::GlobMatcher>, String> {
    match glob_pattern {
        Some(glob) => Ok(Some(
            globset::Glob::new(glob)
                .map_err(|e| format!("Invalid glob '{glob}': {e}"))?
                .compile_matcher(),
        )),
        None => Ok(None),
    }
}

fn file_search_walk_builder(root: &Path) -> ignore::WalkBuilder {
    let mut walk_builder = ignore::WalkBuilder::new(root);
    let root_gitignore = root.join(".gitignore");
    if root_gitignore.exists() {
        walk_builder.add_ignore(&root_gitignore);
    }
    walk_builder
}

fn grep_entry_matches(
    entry: &ignore::DirEntry,
    root: &Path,
    re: &regex::Regex,
    glob_matcher: Option<&globset::GlobMatcher>,
    type_exts: Option<&[&str]>,
) -> Option<(PathBuf, Vec<String>, Vec<usize>)> {
    if !entry
        .file_type()
        .map(|kind| kind.is_file())
        .unwrap_or(false)
    {
        return None;
    }
    let path = entry.path();
    let rel = path.strip_prefix(root).unwrap_or(path);
    if !grep_path_matches(path, rel, glob_matcher, type_exts) {
        return None;
    }
    if path.metadata().ok()?.len() > 5 * 1024 * 1024 {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let matched_line_indices = grep_matched_line_indices(&lines, re);
    if matched_line_indices.is_empty() {
        return None;
    }
    Some((rel.to_path_buf(), lines, matched_line_indices))
}

fn grep_path_matches(
    path: &Path,
    rel: &Path,
    glob_matcher: Option<&globset::GlobMatcher>,
    type_exts: Option<&[&str]>,
) -> bool {
    if let Some(matcher) = glob_matcher {
        if !matcher.is_match(rel) {
            return false;
        }
    }
    if let Some(exts) = type_exts {
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !exts.contains(&ext) {
            return false;
        }
    }
    true
}

fn grep_matched_line_indices(lines: &[String], re: &regex::Regex) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| re.is_match(line).then_some(idx))
        .collect()
}

fn format_grep_results(request: &GrepRequest<'_>, root: &Path, results: GrepResults) -> String {
    if results.lines.is_empty() {
        return format!(
            "No matches for pattern `{}` in {} (gitignore-aware).",
            request.pattern,
            root.display()
        );
    }

    let mut output = results.lines.join("\n");
    if results.hit_cap {
        output.push_str(&format!(
            "\n… (truncated at head_limit={}; matched {} times across {} files so far)",
            request.head_limit, results.total_matches, results.files_with_matches
        ));
    } else {
        output.push_str(&format!(
            "\n[{} match{} across {} file{}]",
            results.total_matches,
            if results.total_matches == 1 { "" } else { "es" },
            results.files_with_matches,
            if results.files_with_matches == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    output
}

pub(crate) async fn tool_glob(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let request = parse_glob_request(input)?;
    let root =
        resolve_file_path_for_caller(request.raw_path, workspace_root, kernel, caller_agent_id)?;
    let glob = compile_glob_matcher(request.pattern)?;
    let hits = collect_glob_hits(&root, &glob);

    Ok(format_glob_results(&request, &root, hits))
}

fn parse_glob_request(input: &serde_json::Value) -> Result<GlobRequest<'_>, String> {
    let pattern = input["pattern"]
        .as_str()
        .ok_or_else(|| "Missing 'pattern' parameter".to_string())?;
    Ok(GlobRequest {
        pattern,
        raw_path: input["path"].as_str().unwrap_or("."),
        head_limit: input["head_limit"].as_u64().unwrap_or(1000) as usize,
    })
}

fn compile_glob_matcher(pattern: &str) -> Result<globset::GlobMatcher, String> {
    Ok(globset::Glob::new(pattern)
        .map_err(|e| format!("Invalid glob '{pattern}': {e}"))?
        .compile_matcher())
}

fn collect_glob_hits(root: &Path, glob: &globset::GlobMatcher) -> Vec<(SystemTime, PathBuf)> {
    let mut hits: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in file_search_walk_builder(root).build().flatten() {
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path);
        if !glob.is_match(rel) {
            continue;
        }
        let mtime = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        hits.push((mtime, rel.to_path_buf()));
    }
    hits
}

fn format_glob_results(
    request: &GlobRequest<'_>,
    root: &Path,
    mut hits: Vec<(SystemTime, PathBuf)>,
) -> String {
    if hits.is_empty() {
        return format!(
            "No files matching '{}' in {} (gitignore-aware).",
            request.pattern,
            root.display()
        );
    }

    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    let total = hits.len();
    let truncated = total > request.head_limit;
    let lines: Vec<String> = hits
        .into_iter()
        .take(request.head_limit)
        .map(|(_, path)| path.display().to_string())
        .collect();
    let mut output = lines.join("\n");
    if truncated {
        output.push_str(&format!(
            "\n… (truncated at head_limit={}; {total} total matches)",
            request.head_limit
        ));
    } else {
        output.push_str(&format!(
            "\n[{total} match{}]",
            if total == 1 { "" } else { "es" }
        ));
    }
    output
}

pub(crate) async fn tool_file_inspect_batch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let request = parse_file_inspect_batch_request(input)?;
    let mut results = Vec::with_capacity(request.operations.len());
    for (idx, op) in request.operations.iter().enumerate() {
        let action = file_inspect_batch_action(op, idx)?;
        let outcome = run_file_inspect_batch_action(
            &action,
            op,
            workspace_root,
            kernel,
            caller_agent_id,
            request.max_read_chars,
        )
        .await;
        if append_file_inspect_batch_result(
            &mut results,
            idx,
            action,
            outcome,
            request.stop_on_error,
        ) {
            break;
        }
    }

    serialize_file_inspect_batch_results(results)
}

fn parse_file_inspect_batch_request(
    input: &serde_json::Value,
) -> Result<FileInspectBatchRequest<'_>, String> {
    let operations = input["operations"]
        .as_array()
        .ok_or("Missing 'operations' array parameter")?;
    if operations.is_empty() {
        return Err("file_inspect_batch requires at least one operation".to_string());
    }
    if operations.len() > MAX_FILE_INSPECT_BATCH_ITEMS {
        return Err(format!(
            "file_inspect_batch accepts at most {MAX_FILE_INSPECT_BATCH_ITEMS} operations"
        ));
    }
    let max_read_chars = input["max_read_chars"]
        .as_u64()
        .unwrap_or(12_000)
        .clamp(500, 50_000) as usize;

    Ok(FileInspectBatchRequest {
        operations,
        max_read_chars,
        stop_on_error: input["stop_on_error"].as_bool().unwrap_or(false),
    })
}

fn file_inspect_batch_action(op: &serde_json::Value, idx: usize) -> Result<String, String> {
    op["action"]
        .as_str()
        .ok_or_else(|| format!("operations[{idx}] missing 'action'"))
        .map(|action| action.trim().to_ascii_lowercase())
}

async fn run_file_inspect_batch_action(
    action: &str,
    op: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    max_read_chars: usize,
) -> Result<String, String> {
    match action {
        "glob" => tool_glob(op, workspace_root, kernel, caller_agent_id).await,
        "grep" => tool_grep(op, workspace_root, kernel, caller_agent_id).await,
        "read" | "file_read" => {
            match tool_file_read(op, workspace_root, kernel, caller_agent_id).await {
                Ok(text) => Ok(truncate_owned(&text, max_read_chars)),
                Err(err) => Err(err),
            }
        }
        "list" | "file_list" => tool_file_list(op, workspace_root, kernel, caller_agent_id).await,
        other => Err(format!(
            "Unsupported file_inspect_batch action '{other}'. Use glob, grep, read, or list."
        )),
    }
}

fn append_file_inspect_batch_result(
    results: &mut Vec<serde_json::Value>,
    idx: usize,
    action: String,
    outcome: Result<String, String>,
    stop_on_error: bool,
) -> bool {
    match outcome {
        Ok(output) => {
            results.push(serde_json::json!({
                "index": idx,
                "action": action,
                "success": true,
                "output": output,
            }));
            false
        }
        Err(error) => {
            results.push(serde_json::json!({
                "index": idx,
                "action": action,
                "success": false,
                "error": error,
            }));
            stop_on_error
        }
    }
}

fn serialize_file_inspect_batch_results(results: Vec<serde_json::Value>) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "success": results.iter().all(|result| result["success"].as_bool() == Some(true)),
        "tool": "file_inspect_batch",
        "operations_executed": results.len(),
        "results": results,
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn emit_grep_result(
    output_mode: &str,
    results: &mut Vec<String>,
    rel: &Path,
    lines: &[String],
    matched_line_indices: &[usize],
    before: usize,
    after: usize,
) -> Result<(), String> {
    match output_mode {
        "files_with_matches" => results.push(rel.display().to_string()),
        "count" => results.push(format!("{}:{}", rel.display(), matched_line_indices.len())),
        "content" => {
            let mut emitted = BTreeSet::new();
            for &matched_idx in matched_line_indices {
                let lo = matched_idx.saturating_sub(before);
                let hi = (matched_idx + after).min(lines.len().saturating_sub(1));
                for line_idx in lo..=hi {
                    emitted.insert(line_idx);
                }
            }
            for line_idx in emitted {
                let marker = if matched_line_indices.contains(&line_idx) {
                    ":"
                } else {
                    "-"
                };
                results.push(format!(
                    "{}{}{}:{}",
                    rel.display(),
                    marker,
                    line_idx + 1,
                    lines[line_idx]
                ));
            }
        }
        other => {
            return Err(format!(
                "Invalid output_mode '{other}'. Use 'content', 'files_with_matches' or 'count'."
            ));
        }
    }
    Ok(())
}

/// ripgrep-style file type aliases (subset).
fn grep_type_extensions(file_type: &str) -> &'static [&'static str] {
    match file_type {
        "rust" | "rs" => &["rs"],
        "ts" | "tsx" => &["ts", "tsx"],
        "js" | "jsx" => &["js", "jsx", "mjs", "cjs"],
        "py" | "python" => &["py", "pyi"],
        "go" => &["go"],
        "java" => &["java"],
        "c" => &["c", "h"],
        "cpp" | "c++" => &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        "md" | "markdown" => &["md", "markdown"],
        "toml" => &["toml"],
        "yaml" | "yml" => &["yaml", "yml"],
        "json" => &["json"],
        "html" => &["html", "htm"],
        "css" => &["css", "scss", "sass"],
        "sh" | "shell" => &["sh", "bash", "zsh"],
        _ => &[],
    }
}
