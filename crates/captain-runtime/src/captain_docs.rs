//! Captain self-documentation index.
//!
//! Captain consults its own tool documentation through the `captain_docs`
//! tool (C.2). The actual prose lives in `docs/captain-tools/<family>.md`
//! under the workspace root, organised by tool family. This module owns the
//! single source of truth for which families exist, so:
//!
//! - C.2 can list / search the right files,
//! - D.1..D.14 can populate them one at a time without risk of drift,
//! - a missing file (someone deletes a stub during a refactor) surfaces as a
//!   test failure rather than a silent gap when Captain queries the doc.
//!
//! Adding a family means: append a `(slug, "phase tag")` entry to
//! `FAMILIES` and create the matching markdown file.

pub use crate::captain_docs_catalog::*;

/// Normalize common user/model aliases to canonical family slugs.
///
/// Keep this intentionally conservative: a wrong family alias can be more
/// expensive than an empty result because it teaches the model the wrong rail.
pub fn normalize_family_filter(family: &str, query: &str) -> Option<&'static str> {
    let normalized = family.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    if normalized.is_empty() {
        return None;
    }
    if let Some((slug, _)) = FAMILIES.iter().find(|(slug, _)| *slug == normalized) {
        return Some(*slug);
    }

    match normalized.as_str() {
        "changelog" | "runtime" | "release-notes" | "release-note" | "releases" | "updates" => {
            Some("runtime-changelog")
        }
        "docs" | "documentation" => {
            let query_lc = query.to_ascii_lowercase();
            if [
                "runtime",
                "changelog",
                "change log",
                "release",
                "update",
                "mise a jour",
                "mise à jour",
            ]
            .iter()
            .any(|needle| query_lc.contains(needle))
            {
                Some("runtime-changelog")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Render exact live schemas for a docs family from `builtin_tool_definitions`.
///
/// This is intentionally generated at runtime instead of hand-written in the
/// Markdown because schema drift is one of the easiest ways to make the agent
/// confidently call the wrong parameters.
pub fn render_live_tool_contracts(slug: &str) -> Option<String> {
    let family_tools = family_tools(slug)?;
    let defs = crate::tool_runner::builtin_tool_definitions();
    let mut out = String::from(
        "\n\n## Live Tool Schemas\n\nGenerated from the running `builtin_tool_definitions()` registry. Prefer these exact parameters over prose if they disagree. Frozen surfaces are omitted from the active schema block.\n",
    );
    for name in family_tools {
        if !crate::surface_gates::tool_is_discoverable_by_default(name) {
            continue;
        }
        let Some(def) = defs.iter().find(|t| t.name == *name) else {
            continue;
        };
        out.push_str("\n### `");
        out.push_str(&def.name);
        out.push_str("`\n\n");
        out.push_str(&def.description);
        out.push_str("\n\n```json\n");
        match serde_json::to_string_pretty(&def.input_schema) {
            Ok(schema) => out.push_str(&schema),
            Err(_) => out.push_str(&def.input_schema.to_string()),
        }
        out.push_str("\n```\n");
    }
    Some(out)
}

fn full_family_body(slug: &str, body: &str) -> String {
    match render_live_tool_contracts(slug) {
        Some(contracts) => {
            let mut full = String::with_capacity(body.len() + contracts.len());
            full.push_str(body);
            full.push_str(&contracts);
            full
        }
        None => body.to_string(),
    }
}

fn runtime_changelog_latest_entry(body: &str) -> Option<String> {
    const LATEST_CHANGELOG_CAP: usize = 3_800;

    let versioned_start = body.find("## Versioned Entries")?;
    let after_versioned = &body[versioned_start..];
    let first_entry_rel = after_versioned.find("\n### ")?;
    let first_entry_start = versioned_start + first_entry_rel + 1;
    let after_first_heading = &body[first_entry_start + 1..];
    let next_entry_rel = after_first_heading.find("\n### ");
    let first_entry_end = next_entry_rel
        .map(|idx| first_entry_start + 1 + idx)
        .unwrap_or(body.len());

    let mut out = String::from("# Runtime changelog family\n\n## Versioned Entries\n\n");
    out.push_str(body[first_entry_start..first_entry_end].trim());
    if out.len() > LATEST_CHANGELOG_CAP {
        let mut cut = LATEST_CHANGELOG_CAP;
        while cut > 0 && !out.is_char_boundary(cut) {
            cut -= 1;
        }
        if let Some(line_cut) = out[..cut].rfind('\n') {
            cut = line_cut;
        }
        out.truncate(cut);
        out.push_str("\n\n[latest entry truncated]");
    }
    Some(out)
}

fn wants_latest_runtime_changelog(query: &str) -> bool {
    let query_lc = query.to_ascii_lowercase();
    [
        "latest",
        "last",
        "recent",
        "current",
        "derniere",
        "dernière",
        "derniere entree",
        "dernière entrée",
        "nouveaute",
        "nouveauté",
    ]
    .iter()
    .any(|needle| query_lc.contains(needle))
}

fn family_body_for_query(slug: &str, body: &str, query: &str) -> String {
    if slug == "runtime-changelog" && wants_latest_runtime_changelog(query) {
        return runtime_changelog_latest_entry(body)
            .unwrap_or_else(|| full_family_body(slug, body));
    }
    full_family_body(slug, body)
}

/// C.2 — Search the bundled audit prose for matches against `query` and
/// optionally restrict to one `family`. Returns a JSON-serializable list
/// of `{family, snippet}` hits sorted by score (most relevant first).
///
/// Search is case-insensitive AND across whitespace-separated query terms.
/// Each hit's `snippet` is the first ~600 characters of the family body
/// after the first matching word, so Captain gets enough context to
/// proceed without paying for the entire file.
pub fn search_family_docs(
    query: &str,
    family: Option<&str>,
    max_results: usize,
) -> Vec<(&'static str, String)> {
    let query_lc = query.to_lowercase();
    let terms: Vec<&str> = query_lc.split_whitespace().collect();

    let mut hits: Vec<(&'static str, String, usize)> = Vec::new();
    for (slug, body) in FAMILY_DOCS {
        if let Some(filter) = family {
            if *slug != filter {
                continue;
            }
        }
        let body_lc = body.to_lowercase();
        if terms.is_empty() || family.is_some() {
            // No query terms or explicit family filter: return whole body.
            hits.push((*slug, family_body_for_query(slug, body, query), 0));
            continue;
        }
        if !terms.iter().all(|t| body_lc.contains(t)) {
            continue;
        }
        let first_pos = terms
            .iter()
            .filter_map(|t| body_lc.find(t))
            .min()
            .unwrap_or(0);
        let snippet_start = first_pos.saturating_sub(80);
        let snippet_end = (first_pos + 600).min(body.len());
        let mut snippet = String::new();
        if snippet_start > 0 {
            snippet.push('…');
        }
        snippet.push_str(&body[snippet_start..snippet_end]);
        if snippet_end < body.len() {
            snippet.push('…');
        }
        let score = terms.iter().map(|t| body_lc.matches(t).count()).sum();
        hits.push((*slug, snippet, score));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.2));
    hits.truncate(max_results);
    hits.into_iter().map(|(s, sn, _)| (s, sn)).collect()
}
