//! Capability preflight for multimodal input.
//!
//! Images stay on the active conversation model. Captain never delegates them
//! to another agent or provider implicitly; an incompatible active model gets
//! an actionable error before the LLM request starts.

use captain_types::error::CaptainError;
use captain_types::message::ContentBlock;

/// Capability required to process the given content blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCapability {
    Vision,
}

/// Check what capabilities are needed for the given content blocks.
pub fn detect_required_capabilities(
    content_blocks: Option<&[ContentBlock]>,
) -> Vec<RequiredCapability> {
    let mut caps = Vec::new();
    if content_blocks
        .unwrap_or_default()
        .iter()
        .any(|block| matches!(block, ContentBlock::Image { .. }))
    {
        caps.push(RequiredCapability::Vision);
    }
    caps
}

fn catalog_provider(provider: &str) -> &str {
    match provider.to_ascii_lowercase().as_str() {
        "openai-codex" => "codex",
        "google" => "gemini",
        "azure-openai" => "azure",
        "kimi" | "kimi2" => "moonshot",
        "dashscope" | "model_studio" => "qwen",
        "copilot" => "github-copilot",
        _ => provider,
    }
}

fn catalog_model_id(provider: &str, model_id: &str) -> String {
    let model_slug = model_id
        .split_once('/')
        .filter(|(prefix, _)| catalog_provider(prefix).eq_ignore_ascii_case(provider))
        .map(|(_, slug)| slug)
        .unwrap_or(model_id);
    format!("{provider}/{model_slug}")
}

/// Check if the active model supports the given capability.
pub fn model_supports(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    provider: &str,
    model_id: &str,
    cap: RequiredCapability,
) -> bool {
    let provider = catalog_provider(provider);
    let canonical_id = catalog_model_id(provider, model_id);
    let model = catalog
        .find_model(&canonical_id)
        .filter(|model| model.provider.eq_ignore_ascii_case(provider))
        .or_else(|| {
            catalog
                .find_model(model_id)
                .filter(|model| model.provider.eq_ignore_ascii_case(provider))
        });

    match cap {
        RequiredCapability::Vision => model.is_some_and(|model| model.supports_vision),
    }
}

fn active_model_label(provider: &str, model_id: &str) -> String {
    if model_id.contains('/') {
        model_id.to_string()
    } else {
        format!("{provider}/{model_id}")
    }
}

/// Reject unsupported multimodal input without changing agent or provider.
pub fn ensure_active_model_supports(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    provider: &str,
    model_id: &str,
    content_blocks: Option<&[ContentBlock]>,
) -> Result<(), CaptainError> {
    for capability in detect_required_capabilities(content_blocks) {
        if model_supports(catalog, provider, model_id, capability) {
            continue;
        }

        let model = active_model_label(provider, model_id);
        return Err(match capability {
            RequiredCapability::Vision => CaptainError::CapabilityDenied(format!(
                "active model '{model}' does not support image input. Captain did not send the image to another agent or provider. Switch this agent to a vision-capable model, then retry"
            )),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> ContentBlock {
        ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        }
    }

    #[test]
    fn text_only_needs_no_capability() {
        let text = ContentBlock::Text {
            text: "hello".to_string(),
            provider_metadata: None,
        };
        assert!(detect_required_capabilities(None).is_empty());
        assert!(detect_required_capabilities(Some(&[])).is_empty());
        assert!(detect_required_capabilities(Some(&[text])).is_empty());
    }

    #[test]
    fn image_requires_vision() {
        assert_eq!(
            detect_required_capabilities(Some(&[image()])),
            vec![RequiredCapability::Vision]
        );
    }

    #[test]
    fn codex_aliases_keep_images_on_the_active_model() {
        let catalog = captain_runtime::model_catalog::ModelCatalog::new();

        for (provider, model) in [
            ("codex", "gpt-5.5"),
            ("openai-codex", "gpt-5.5"),
            ("openai-codex", "openai-codex/gpt-5.5"),
            ("openai-codex", "codex/gpt-5.5"),
        ] {
            assert!(model_supports(
                &catalog,
                provider,
                model,
                RequiredCapability::Vision
            ));
            ensure_active_model_supports(&catalog, provider, model, Some(&[image()]))
                .expect("the active Codex model should receive the image directly");
        }
    }

    #[test]
    fn multimodal_provider_aliases_resolve_to_their_catalog_family() {
        let catalog = captain_runtime::model_catalog::ModelCatalog::new();

        for (provider, model) in [("google", "gemini-2.5-flash"), ("azure-openai", "gpt-4o")] {
            assert!(model_supports(
                &catalog,
                provider,
                model,
                RequiredCapability::Vision
            ));
        }
    }

    #[test]
    fn incompatible_model_is_rejected_without_hidden_delegation() {
        let catalog = captain_runtime::model_catalog::ModelCatalog::new();
        let error = ensure_active_model_supports(
            &catalog,
            "codex",
            "gpt-5.3-codex-spark",
            Some(&[image()]),
        )
        .expect_err("Spark is text-only in the catalog");
        let message = error.to_string();

        assert!(message.contains("does not support image input"));
        assert!(message.contains("did not send the image to another agent or provider"));
        assert!(message.contains("Switch this agent to a vision-capable model"));
    }

    #[test]
    fn unknown_model_does_not_claim_image_support() {
        let catalog = captain_runtime::model_catalog::ModelCatalog::new();
        assert!(!model_supports(
            &catalog,
            "custom-provider",
            "unknown-model",
            RequiredCapability::Vision
        ));
    }
}
