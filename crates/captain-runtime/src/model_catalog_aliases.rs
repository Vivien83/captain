use std::collections::HashMap;

pub(crate) fn builtin_aliases() -> HashMap<String, String> {
    let pairs = [
        ("sonnet", "claude-sonnet-4-6"),
        ("claude-sonnet", "claude-sonnet-4-6"),
        ("haiku", "claude-haiku-4-5-20251001"),
        ("claude-haiku", "claude-haiku-4-5-20251001"),
        ("opus", "claude-opus-4-6"),
        ("claude-opus", "claude-opus-4-6"),
        ("gpt4", "gpt-4o"),
        ("gpt4o", "gpt-4o"),
        ("gpt4-mini", "gpt-4o-mini"),
        ("gpt5", "gpt-5.2"),
        ("gpt5-mini", "gpt-5-mini"),
        ("flash", "gemini-2.5-flash"),
        ("gemini-pro", "gemini-3.1-pro-preview"),
        ("gemini-flash", "gemini-3-flash-preview"),
        ("deepseek", "deepseek-chat"),
        ("llama", "llama-3.3-70b-versatile"),
        ("llama-70b", "llama-3.3-70b-versatile"),
        ("mixtral", "mixtral-8x7b-32768"),
        ("mistral", "mistral-large-latest"),
        ("codestral", "codestral-latest"),
        ("deepseek-v3", "deepseek-chat"),
        ("deepseek-r1", "deepseek-reasoner"),
        ("mistral-nemo", "open-mistral-nemo"),
        ("pixtral", "pixtral-large-latest"),
        ("grok", "grok-4-0709"),
        ("grok-4", "grok-4-0709"),
        ("grok-mini", "grok-2-mini"),
        ("grok3", "grok-3"),
        ("grok-fast", "grok-4-1-fast-reasoning"),
        ("sonar", "sonar-pro"),
        ("jamba", "jamba-1.5-large"),
        ("command-r", "command-r-plus"),
        ("command", "command-a"),
        ("copilot", "copilot/gpt-4o"),
        ("copilot-4o", "copilot/gpt-4o"),
        ("copilot-4", "copilot/gpt-4"),
        ("copilot-gpt4o", "copilot/gpt-4o"),
        ("copilot-gpt4", "copilot/gpt-4"),
        ("qwen", "qwen-plus"),
        ("glm", "glm-5-20250605"),
        ("ernie", "ernie-4.5-8k"),
        ("kimi", "kimi-k2"),
        ("moonshot", "moonshot-v1-128k"),
        ("minimax", "MiniMax-M2.5"),
        ("minimax-m2.5", "MiniMax-M2.5"),
        ("minimax-m2.5-highspeed", "MiniMax-M2.5-highspeed"),
        ("minimax-highspeed", "MiniMax-M2.5-highspeed"),
        ("minimax-m2.1", "MiniMax-M2.1"),
        ("codegeex", "codegeex-4"),
        ("codex", "codex/gpt-5.5"),
        ("codex-5.5", "codex/gpt-5.5"),
        ("codex-5.4", "codex/gpt-5.4"),
        ("codex-5.3", "codex/gpt-5.3-codex"),
        ("codex-5.3-codex", "codex/gpt-5.3-codex"),
        ("codex-5.3-spark", "codex/gpt-5.3-codex-spark"),
        ("codex-5.3-codex-spark", "codex/gpt-5.3-codex-spark"),
        ("codex-5.2", "codex/gpt-5.2"),
        ("codex-4.1", "codex/gpt-4.1"),
        ("nemotron", "nvidia/llama-3.1-nemotron-70b-instruct"),
        ("venice", "venice-uncensored"),
        ("claude-code", "claude-code/sonnet"),
        ("claude-code-opus", "claude-code/opus"),
        ("claude-code-sonnet", "claude-code/sonnet"),
        ("claude-code-haiku", "claude-code/haiku"),
        ("qwen-code", "qwen-code/qwen3-coder"),
        ("qwen-coder", "qwen-code/qwen3-coder"),
        ("qwen-coder-plus", "qwen-code/qwen-coder-plus"),
        ("qwq", "qwen-code/qwq-32b"),
        (
            "openrouter/free",
            "openrouter/meta-llama/llama-3.1-8b-instruct:free",
        ),
        ("free", "openrouter/meta-llama/llama-3.1-8b-instruct:free"),
        ("free-reasoning", "openrouter/deepseek/deepseek-r1:free"),
    ];
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_lowercase(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_alias_count_is_stable() {
        assert_eq!(builtin_aliases().len(), 71);
    }

    #[test]
    fn core_aliases_cover_primary_provider_shortcuts() {
        let aliases = builtin_aliases();
        assert_eq!(aliases.get("sonnet").unwrap(), "claude-sonnet-4-6");
        assert_eq!(aliases.get("gpt5").unwrap(), "gpt-5.2");
        assert_eq!(aliases.get("gemini-pro").unwrap(), "gemini-3.1-pro-preview");
        assert_eq!(aliases.get("grok").unwrap(), "grok-4-0709");
        assert_eq!(aliases.get("jamba").unwrap(), "jamba-1.5-large");
    }

    #[test]
    fn code_provider_aliases_keep_cli_defaults() {
        let aliases = builtin_aliases();
        assert_eq!(aliases.get("codex").unwrap(), "codex/gpt-5.5");
        assert_eq!(aliases.get("claude-code").unwrap(), "claude-code/sonnet");
        assert_eq!(aliases.get("qwen-code").unwrap(), "qwen-code/qwen3-coder");
    }

    #[test]
    fn alias_keys_are_normalized_lowercase() {
        for key in builtin_aliases().keys() {
            assert_eq!(key, &key.to_lowercase());
        }
    }
}
