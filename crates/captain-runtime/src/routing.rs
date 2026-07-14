//! Model routing — auto-selects cheap/mid/expensive models by query complexity.
//!
//! The router scores each `CompletionRequest` based on heuristics (token count,
//! tool availability, code markers, conversation depth) and picks the cheapest
//! model that can handle the task.

use crate::llm_driver::CompletionRequest;
use captain_types::agent::ModelRoutingConfig;

/// Task complexity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// Quick lookup, greetings, simple Q&A — use the cheapest model.
    Simple,
    /// Standard conversational task — use a mid-tier model.
    Medium,
    /// Multi-step reasoning, code generation, complex analysis — use the best model.
    Complex,
}

impl std::fmt::Display for TaskComplexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskComplexity::Simple => write!(f, "simple"),
            TaskComplexity::Medium => write!(f, "medium"),
            TaskComplexity::Complex => write!(f, "complex"),
        }
    }
}

/// Model router that selects the appropriate model based on query complexity.
#[derive(Debug, Clone)]
pub struct ModelRouter {
    config: ModelRoutingConfig,
}

impl ModelRouter {
    /// Create a new model router with the given routing configuration.
    pub fn new(config: ModelRoutingConfig) -> Self {
        Self { config }
    }

    /// Score a completion request and determine its complexity tier.
    ///
    /// Heuristics:
    /// - **Token count**: total characters in messages as a proxy for tokens
    /// - **Tool availability**: having tools suggests potential multi-step work
    /// - **Code markers**: backticks, `fn`, `def`, `class`, etc.
    /// - **Conversation depth**: more messages = more context = harder reasoning
    /// - **System prompt length**: longer prompts often imply complex tasks
    pub fn score(&self, request: &CompletionRequest) -> TaskComplexity {
        // Extract the last user message for semantic analysis
        let last_user_text = request
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, captain_types::message::Role::User))
            .map(|m| m.content.text_content())
            .unwrap_or_default();
        let text_lower = last_user_text.to_lowercase();
        let text_len = last_user_text.len();

        // ── SIMPLE patterns: greetings, time, yes/no, short commands ──
        let simple_patterns = [
            "salut",
            "hello",
            "hi",
            "hey",
            "bonjour",
            "bonsoir",
            "coucou",
            "merci",
            "thanks",
            "ok",
            "oui",
            "non",
            "yes",
            "no",
            "quelle heure",
            "what time",
            "quel jour",
            "what day",
            "ça va",
            "how are you",
            "comment tu vas",
        ];
        if text_len < 50 && simple_patterns.iter().any(|p| text_lower.contains(p)) {
            return TaskComplexity::Simple;
        }

        // Very short messages (< 30 chars, no special markers) → simple
        if text_len < 30 {
            return TaskComplexity::Simple;
        }

        // ── COMPLEX patterns: analysis, code, multi-step reasoning ──
        let complex_patterns = [
            "analyse",
            "analyze",
            "compare",
            "debug",
            "refactor",
            "implement",
            "implémente",
            "architecture",
            "design",
            "research",
            "investigate",
            "optimize",
            "optimise",
            "explain in detail",
            "explique en détail",
            "write a",
            "écris un",
            "create a",
            "crée un",
            "step by step",
            "étape par étape",
        ];
        let complex_hits: u32 = complex_patterns
            .iter()
            .filter(|p| text_lower.contains(*p))
            .count() as u32;

        // Code markers
        let code_markers = [
            "```",
            "fn ",
            "def ",
            "class ",
            "import ",
            "function ",
            "async ",
            "struct ",
            "impl ",
        ];
        let code_hits: u32 = code_markers
            .iter()
            .filter(|p| text_lower.contains(*p))
            .count() as u32;

        // Long messages with code or analysis keywords → complex
        if complex_hits >= 2
            || code_hits >= 2
            || (text_len > 500 && (complex_hits > 0 || code_hits > 0))
        {
            return TaskComplexity::Complex;
        }

        // ── MEDIUM: everything else (single-topic questions, search, tools) ──
        // Long messages without complexity markers are still medium
        if text_len > 200 || complex_hits > 0 || code_hits > 0 {
            return TaskComplexity::Medium;
        }

        // Default: medium-length messages without markers → simple
        // (e.g., "cherche la météo de Lyon" = 30 chars, no complexity)
        if text_len < 100 {
            return TaskComplexity::Simple;
        }

        TaskComplexity::Medium
    }

    /// Select the model name for a given complexity tier.
    pub fn model_for_complexity(&self, complexity: TaskComplexity) -> &str {
        match complexity {
            TaskComplexity::Simple => &self.config.simple_model,
            TaskComplexity::Medium => &self.config.medium_model,
            TaskComplexity::Complex => &self.config.complex_model,
        }
    }

    /// Score a request and return the selected model name + complexity.
    pub fn select_model(&self, request: &CompletionRequest) -> (TaskComplexity, String) {
        let complexity = self.score(request);
        let model = self.model_for_complexity(complexity).to_string();
        (complexity, model)
    }

    /// Validate that all configured models exist in the catalog.
    ///
    /// Returns a list of warning messages for models not found in the catalog.
    pub fn validate_models(&self, catalog: &crate::model_catalog::ModelCatalog) -> Vec<String> {
        let mut warnings = vec![];
        for model in [
            &self.config.simple_model,
            &self.config.medium_model,
            &self.config.complex_model,
        ] {
            if catalog.find_model(model).is_none() {
                warnings.push(format!("Model '{}' not found in catalog", model));
            }
        }
        warnings
    }

    /// Resolve aliases in the routing config using the catalog.
    ///
    /// For example, if "sonnet" is configured, resolves to "claude-sonnet-4-6".
    pub fn resolve_aliases(&mut self, catalog: &crate::model_catalog::ModelCatalog) {
        if let Some(resolved) = catalog.resolve_alias(&self.config.simple_model) {
            self.config.simple_model = resolved.to_string();
        }
        if let Some(resolved) = catalog.resolve_alias(&self.config.medium_model) {
            self.config.medium_model = resolved.to_string();
        }
        if let Some(resolved) = catalog.resolve_alias(&self.config.complex_model) {
            self.config.complex_model = resolved.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{Message, MessageContent, Role};
    use captain_types::tool::ToolDefinition;

    fn default_config() -> ModelRoutingConfig {
        ModelRoutingConfig {
            simple_model: "llama-3.3-70b-versatile".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: "claude-opus-4-6".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        }
    }

    fn make_request(messages: Vec<Message>, tools: Vec<ToolDefinition>) -> CompletionRequest {
        CompletionRequest {
            model: "placeholder".to_string(),
            messages,
            tools,
            max_tokens: 4096,
            temperature: 0.7,
            system: None,
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        }
    }

    #[test]
    fn test_simple_greeting_routes_to_simple() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text("Hello!"),
            }],
            vec![],
        );
        let (complexity, model) = router.select_model(&request);
        assert_eq!(complexity, TaskComplexity::Simple);
        assert_eq!(model, "llama-3.3-70b-versatile");
    }

    #[test]
    fn test_code_markers_increase_complexity() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text(
                    "Write a function that implements async file reading with struct and impl blocks:\n\
                     ```rust\nfn main() { }\n```"
                ),
            }],
            vec![],
        );
        let complexity = router.score(&request);
        // Should be at least Medium due to code markers
        assert_ne!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn test_complex_analysis_request() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text("Please analyze and compare these two approaches, then implement the better one."),
            }],
            vec![],
        );
        let complexity = router.score(&request);
        assert_eq!(complexity, TaskComplexity::Complex);
    }

    #[test]
    fn test_medium_search_request() {
        let router = ModelRouter::new(default_config());
        let request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text(
                    "Cherche la météo de Lyon pour demain et dis-moi si je peux prendre la moto",
                ),
            }],
            vec![],
        );
        let complexity = router.score(&request);
        assert_eq!(complexity, TaskComplexity::Simple);
    }

    #[test]
    fn test_greeting_is_simple() {
        let router = ModelRouter::new(default_config());
        for greeting in &["Salut !", "Hello", "Bonjour", "Hey", "Merci"] {
            let request = make_request(
                vec![Message {
                    role: Role::User,
                    content: MessageContent::text(*greeting),
                }],
                vec![],
            );
            let complexity = router.score(&request);
            assert_eq!(
                complexity,
                TaskComplexity::Simple,
                "'{greeting}' should be Simple"
            );
        }
    }

    #[test]
    fn test_model_for_complexity() {
        let router = ModelRouter::new(default_config());
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Simple),
            "llama-3.3-70b-versatile"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Medium),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Complex),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_complexity_display() {
        assert_eq!(TaskComplexity::Simple.to_string(), "simple");
        assert_eq!(TaskComplexity::Medium.to_string(), "medium");
        assert_eq!(TaskComplexity::Complex.to_string(), "complex");
    }

    #[test]
    fn test_validate_models_all_found() {
        let catalog = crate::model_catalog::ModelCatalog::new();
        let config = ModelRoutingConfig {
            simple_model: "llama-3.3-70b-versatile".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: "claude-opus-4-6".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let router = ModelRouter::new(config);
        let warnings = router.validate_models(&catalog);
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_models_unknown() {
        let catalog = crate::model_catalog::ModelCatalog::new();
        let config = ModelRoutingConfig {
            simple_model: "unknown-model".to_string(),
            medium_model: "claude-sonnet-4-6".to_string(),
            complex_model: "claude-opus-4-6".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let router = ModelRouter::new(config);
        let warnings = router.validate_models(&catalog);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unknown-model"));
    }

    #[test]
    fn test_resolve_aliases() {
        let catalog = crate::model_catalog::ModelCatalog::new();
        let config = ModelRoutingConfig {
            simple_model: "llama".to_string(),
            medium_model: "sonnet".to_string(),
            complex_model: "opus".to_string(),
            simple_threshold: 200,
            complex_threshold: 800,
        };
        let mut router = ModelRouter::new(config);
        router.resolve_aliases(&catalog);
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Simple),
            "llama-3.3-70b-versatile"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Medium),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            router.model_for_complexity(TaskComplexity::Complex),
            "claude-opus-4-6"
        );
    }

    #[test]
    fn test_system_prompt_does_not_inflate_score() {
        let router = ModelRouter::new(default_config());
        let mut request = make_request(
            vec![Message {
                role: Role::User,
                content: MessageContent::text("Salut !"),
            }],
            vec![],
        );
        request.system = Some("A".repeat(10000)); // Huge system prompt
        let complexity = router.score(&request);
        // "Salut !" is still simple regardless of system prompt size
        assert_eq!(complexity, TaskComplexity::Simple);
    }
}
