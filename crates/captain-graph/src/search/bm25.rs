//! BM25+ inverted index — zero dependency, stdlib-only tokenization.
//!
//! Indexes entity text (name + string properties) for full-text search.
//! IDF is lazily recomputed at query time when the index is dirty.

use std::collections::HashMap;

use crate::core::types::EntityId;
use crate::search::SearchHit;

// ── Stop words (English, compact) ─────────────────────────────────

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "do", "for", "from", "had", "has",
    "have", "he", "her", "him", "his", "how", "if", "in", "into", "is", "it", "its", "just", "me",
    "my", "no", "not", "of", "on", "or", "our", "out", "she", "so", "than", "that", "the", "them",
    "then", "there", "these", "they", "this", "to", "up", "us", "was", "we", "were", "what",
    "when", "which", "who", "will", "with", "you", "your",
];

fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.binary_search(&word).is_ok()
}

// ── Tokenizer ─────────────────────────────────────────────────────

/// Tokenize text into lowercase alphanumeric terms, filtering stop words and single chars.
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1 && !is_stop_word(t))
        .map(String::from)
        .collect()
}

// ── BM25 Index ────────────────────────────────────────────────────

/// A posting: which document (entity) contains this term, and how many times.
#[derive(Debug, Clone)]
struct Posting {
    doc_id: u32,
    tf: u32,
    doc_len: u32,
}

/// BM25+ inverted index for full-text search over entities.
pub struct Bm25Index {
    /// term → list of postings (sorted by doc_id for fast lookup)
    postings: HashMap<String, Vec<Posting>>,
    /// doc_id → document length (in tokens)
    doc_lengths: HashMap<u32, u32>,
    /// Total token count across all documents (for avgdl)
    total_tokens: u64,
    /// Cached IDF values per term
    idf_cache: HashMap<String, f64>,
    /// Whether the IDF cache needs recomputing
    dirty: bool,
    /// BM25 parameters
    k1: f64,
    b: f64,
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new()
    }
}

impl Bm25Index {
    /// Create a new empty BM25 index with default parameters (k1=1.2, b=0.75).
    pub fn new() -> Self {
        Self {
            postings: HashMap::new(),
            doc_lengths: HashMap::new(),
            total_tokens: 0,
            idf_cache: HashMap::new(),
            dirty: false,
            k1: 1.2,
            b: 0.75,
        }
    }

    /// Number of indexed documents.
    pub fn doc_count(&self) -> usize {
        self.doc_lengths.len()
    }

    /// Index a document (entity). Extracts tokens from the given text.
    /// If the document was already indexed, it is re-indexed (old data removed first).
    pub fn index_document(&mut self, doc_id: u32, text: &str) {
        // Remove old data if re-indexing
        if self.doc_lengths.contains_key(&doc_id) {
            self.remove_document(doc_id);
        }

        let tokens = tokenize(text);
        let doc_len = tokens.len() as u32;

        if doc_len == 0 {
            return;
        }

        // Count term frequencies
        let mut tf_map: HashMap<&str, u32> = HashMap::new();
        for token in &tokens {
            *tf_map.entry(token.as_str()).or_default() += 1;
        }

        // Insert postings
        for (term, tf) in tf_map {
            let posting_list = self.postings.entry(term.to_string()).or_default();
            posting_list.push(Posting {
                doc_id,
                tf,
                doc_len,
            });
        }

        self.doc_lengths.insert(doc_id, doc_len);
        self.total_tokens += doc_len as u64;
        self.dirty = true;
    }

    /// Remove a document from the index.
    pub fn remove_document(&mut self, doc_id: u32) {
        if let Some(len) = self.doc_lengths.remove(&doc_id) {
            self.total_tokens -= len as u64;

            // Remove from all posting lists
            self.postings.retain(|_, postings| {
                postings.retain(|p| p.doc_id != doc_id);
                !postings.is_empty()
            });

            self.dirty = true;
        }
    }

    /// Search the index for the given query text. Returns top-k results scored by BM25+.
    pub fn search(&mut self, query: &str, k: usize) -> Vec<SearchHit> {
        if k == 0 || self.doc_lengths.is_empty() {
            return Vec::new();
        }

        // Recompute IDF if dirty
        if self.dirty {
            self.recompute_idf();
        }

        let query_tokens = tokenize(query);
        if query_tokens.is_empty() {
            return Vec::new();
        }

        let n = self.doc_lengths.len() as f64;
        let avgdl = if n > 0.0 {
            self.total_tokens as f64 / n
        } else {
            1.0
        };

        // Accumulate scores per document using dense Vec (O(1) access, no hashing)
        let max_doc_id = self.doc_lengths.keys().copied().max().unwrap_or(0) as usize;
        let mut scores = vec![0.0_f64; max_doc_id + 1];

        for token in &query_tokens {
            let idf = match self.idf_cache.get(token.as_str()) {
                Some(&v) => v,
                None => continue, // term not in corpus
            };

            let postings = match self.postings.get(token.as_str()) {
                Some(p) => p,
                None => continue,
            };

            for posting in postings {
                let dl = posting.doc_len as f64;
                let tf = posting.tf as f64;

                // BM25+ formula
                let numerator = tf * (self.k1 + 1.0);
                let denominator = tf + self.k1 * (1.0 - self.b + self.b * dl / avgdl);
                let score = idf * numerator / denominator;

                scores[posting.doc_id as usize] += score;
            }
        }

        // Top-k extraction: collect only non-zero scores
        let mut scored: Vec<(u32, f64)> = scores
            .iter()
            .enumerate()
            .filter(|(_, &s)| s > 0.0)
            .map(|(id, &s)| (id as u32, s))
            .collect();

        if k < scored.len() {
            scored.select_nth_unstable_by(k, |a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(k);
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .map(|(doc_id, score)| SearchHit {
                entity_id: EntityId(doc_id as u64),
                score: score as f32,
            })
            .collect()
    }

    /// Recompute IDF for all terms.
    fn recompute_idf(&mut self) {
        let n = self.doc_lengths.len() as f64;
        self.idf_cache.clear();

        for (term, postings) in &self.postings {
            let df = postings.len() as f64;
            // IDF formula: ln((N - df + 0.5) / (df + 0.5) + 1)
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            self.idf_cache.insert(term.clone(), idf);
        }

        self.dirty = false;
    }
}

// ── Helpers for HoraCore integration ──────────────────────────────

/// Build indexable text from an entity's name and string properties.
pub fn entity_text(name: &str, properties: &crate::core::types::Properties) -> String {
    let mut text = name.to_string();
    for (key, value) in properties {
        if let crate::core::types::PropertyValue::String(s) = value {
            text.push(' ');
            text.push_str(key);
            text.push(' ');
            text.push_str(s);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("Hello, World! This is a test.");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"test".to_string()));
        // Stop words filtered
        assert!(!tokens.contains(&"this".to_string()));
        assert!(!tokens.contains(&"is".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
    }

    #[test]
    fn test_tokenize_single_chars_filtered() {
        let tokens = tokenize("I am a b c developer");
        assert!(!tokens.contains(&"i".to_string()));
        assert!(!tokens.contains(&"b".to_string()));
        assert!(!tokens.contains(&"c".to_string()));
        assert!(tokens.contains(&"am".to_string()));
        assert!(tokens.contains(&"developer".to_string()));
    }

    #[test]
    fn test_tokenize_alphanumeric() {
        let tokens = tokenize("version2 is rust-based v0.1");
        assert!(tokens.contains(&"version2".to_string()));
        assert!(tokens.contains(&"rust".to_string()));
        assert!(tokens.contains(&"based".to_string()));
    }

    #[test]
    fn test_index_and_search_basic() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "Rust programming language systems");
        idx.index_document(2, "Python programming language scripting");
        idx.index_document(3, "hora graph knowledge engine Rust");

        let results = idx.search("Rust", 10);
        assert!(!results.is_empty());
        // Both docs with "rust" should appear
        let ids: Vec<u64> = results.iter().map(|h| h.entity_id.0).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn test_search_tf_matters() {
        let mut idx = Bm25Index::new();
        // Doc 1 has "authentication" twice
        idx.index_document(1, "authentication oauth authentication flow");
        // Doc 2 has it once
        idx.index_document(2, "authentication basic flow");

        let results = idx.search("authentication", 10);
        assert_eq!(results.len(), 2);
        // Doc with more occurrences should score higher
        assert_eq!(results[0].entity_id.0, 1);
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_search_idf_matters() {
        let mut idx = Bm25Index::new();
        // "rare" appears in 1 doc, "common" in all 3
        idx.index_document(1, "rare unique term");
        idx.index_document(2, "common common common");
        idx.index_document(3, "common everywhere common");

        let results = idx.search("rare", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id.0, 1);
    }

    #[test]
    fn test_search_no_match() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "hora graph engine");

        let results = idx.search("nonexistent", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_query() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "hora graph engine");

        let results = idx.search("", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_stopword_only_query() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "hora graph engine");

        let results = idx.search("the a is", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_remove_document() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "hora graph engine");
        idx.index_document(2, "other document");

        idx.remove_document(1);

        let results = idx.search("hora", 10);
        assert!(results.is_empty());
        assert_eq!(idx.doc_count(), 1);
    }

    #[test]
    fn test_reindex_document() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "old content about cats");

        // Re-index with new content
        idx.index_document(1, "new content about dogs");

        let cats = idx.search("cats", 10);
        assert!(cats.is_empty());

        let dogs = idx.search("dogs", 10);
        assert_eq!(dogs.len(), 1);
        assert_eq!(dogs[0].entity_id.0, 1);
    }

    #[test]
    fn test_entity_text_builder() {
        use crate::core::types::{Properties, PropertyValue};

        let mut props = Properties::new();
        props.insert(
            "description".to_string(),
            PropertyValue::String("knowledge graph engine".to_string()),
        );
        props.insert("stars".to_string(), PropertyValue::Int(42)); // non-string, ignored

        let text = entity_text("hora", &props);
        assert!(text.contains("hora"));
        assert!(text.contains("knowledge graph engine"));
        assert!(!text.contains("42"));
    }

    #[test]
    fn test_top_k_limits() {
        let mut idx = Bm25Index::new();
        for i in 0..20 {
            idx.index_document(i, &format!("document number {} about rust", i));
        }

        let results = idx.search("rust", 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_multi_term_query() {
        let mut idx = Bm25Index::new();
        idx.index_document(1, "rust programming language");
        idx.index_document(2, "knowledge graph database");
        idx.index_document(3, "rust graph engine");

        let results = idx.search("rust graph", 10);
        // Doc 3 matches both terms → should rank first
        assert_eq!(results[0].entity_id.0, 3);
    }
}
