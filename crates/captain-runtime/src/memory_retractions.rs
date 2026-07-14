//! Central memory retraction guard.
//!
//! `memory_forget` removes canonical long-term facts, but old snapshots
//! (journals, checkpoints, graph summaries) can still exist as archives.
//! Retractions are the small active guard that says: keep the past, but do
//! not inject forgotten facts back into the current prompt.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MEMORY_RETRACTIONS_KEY: &str = "__captain_memory_retractions_v1";
pub const MAX_RETRACTIONS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRetraction {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub terms: Vec<String>,
    pub created_at: i64,
}

impl MemoryRetraction {
    pub fn from_filters(
        subject: Option<&str>,
        predicate: Option<&str>,
        object: Option<&str>,
    ) -> Option<Self> {
        let terms = derive_terms(subject, predicate, object);
        if terms.is_empty() {
            return None;
        }
        let now = now_ms();
        Some(Self {
            id: format!("rtx-{now}-{}", terms.join("-")),
            subject: subject.map(str::to_string),
            predicate: predicate.map(str::to_string),
            object: object.map(str::to_string),
            terms,
            created_at: now,
        })
    }

    fn same_scope(&self, other: &Self) -> bool {
        self.subject == other.subject
            && self.predicate == other.predicate
            && self.object == other.object
            && self.terms == other.terms
    }
}

pub fn append_retraction(
    mut existing: Vec<MemoryRetraction>,
    retraction: MemoryRetraction,
) -> Vec<MemoryRetraction> {
    if !existing.iter().any(|r| r.same_scope(&retraction)) {
        existing.push(retraction);
    }
    if existing.len() > MAX_RETRACTIONS {
        let drain = existing.len() - MAX_RETRACTIONS;
        existing.drain(0..drain);
    }
    existing
}

pub fn load_retractions(value: Option<Value>) -> Vec<MemoryRetraction> {
    let Some(value) = value else {
        return Vec::new();
    };
    if let Some(items) = value.get("items") {
        serde_json::from_value(items.clone()).unwrap_or_default()
    } else if value.is_array() {
        serde_json::from_value(value).unwrap_or_default()
    } else {
        Vec::new()
    }
}

pub fn retractions_to_value(items: &[MemoryRetraction]) -> Value {
    serde_json::json!({
        "schema": "captain.memory_retractions.v1",
        "items": items,
    })
}

pub fn text_matches_any(text: &str, retractions: &[MemoryRetraction]) -> bool {
    if retractions.is_empty() || text.trim().is_empty() {
        return false;
    }
    let haystack = text.to_lowercase();
    retractions
        .iter()
        .flat_map(|r| r.terms.iter())
        .any(|term| !term.is_empty() && haystack.contains(term))
}

pub fn filter_retracted_lines(text: &str, retractions: &[MemoryRetraction]) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    if retractions.is_empty() {
        return Some(text.to_string());
    }
    let filtered = text
        .lines()
        .filter(|line| !text_matches_any(line, retractions))
        .collect::<Vec<_>>()
        .join("\n");
    if filtered.trim().is_empty() {
        None
    } else {
        Some(filtered)
    }
}

pub fn filter_optional_text(
    text: Option<String>,
    retractions: &[MemoryRetraction],
) -> Option<String> {
    text.and_then(|s| filter_retracted_lines(&s, retractions))
}

fn derive_terms(
    subject: Option<&str>,
    predicate: Option<&str>,
    object: Option<&str>,
) -> Vec<String> {
    let mut terms = extract_terms(object);
    if terms.is_empty() {
        terms.extend(extract_terms(predicate));
    }
    if terms.is_empty() {
        terms.extend(extract_terms(subject));
    }

    let mut seen = HashSet::new();
    terms
        .into_iter()
        .filter(|t| seen.insert(t.clone()))
        .take(8)
        .collect()
}

fn extract_terms(value: Option<&str>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .flat_map(|chunk| chunk.split(['%', '*', '?']))
        .flat_map(|chunk| chunk.split('_'))
        .flat_map(|chunk| chunk.split('-'))
        .map(str::trim)
        .filter(|term| term.len() >= 3)
        .filter(|term| !GENERIC_TERMS.contains(term))
        .map(str::to_string)
        .collect()
}

const GENERIC_TERMS: &[&str] = &[
    "agent",
    "captain",
    "fact",
    "info",
    "memory",
    "object",
    "predicate",
    "preference",
    "preferences",
    "prefers",
    "project",
    "subject",
    "user",
    "utilisateur",
];

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retraction_uses_specific_object_term_first() {
        let r = MemoryRetraction::from_filters(
            Some("user"),
            Some("has_pet"),
            Some("%ancienne_valeur%"),
        )
        .unwrap();
        assert_eq!(r.terms, vec!["ancienne", "valeur"]);
    }

    #[test]
    fn filter_retracted_lines_keeps_archive_context_without_stale_fact() {
        let r = MemoryRetraction::from_filters(None, None, Some("%ancienne_valeur%")).unwrap();
        let input =
            "Utilisateur:\n- aime la mangue\n- ancienne_valeur est obsolet\nCheckpoint: conversation passee";
        let output = filter_retracted_lines(input, &[r]).unwrap();
        assert!(output.contains("mangue"));
        assert!(output.contains("Checkpoint"));
        assert!(!output.to_lowercase().contains("ancienne_valeur"));
    }

    #[test]
    fn generic_subject_only_does_not_create_global_suppression() {
        assert!(MemoryRetraction::from_filters(Some("user"), None, None).is_none());
    }
}
