use crate::kernel_handle::KernelHandle;
use captain_types::agent::AgentManifest;
use captain_types::memory::MemoryFragment;
use std::sync::Arc;

pub(crate) fn prompt_cap_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| idx)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

fn push_compact_section(out: &mut String, title: &str, body: &str, max_chars: usize) {
    let body = body.trim();
    if body.is_empty() {
        return;
    }
    out.push_str("\n\n### ");
    out.push_str(title);
    out.push('\n');
    out.push_str(&prompt_cap_chars(body, max_chars));
}

fn compact_recalled_memory_section(
    memories: &[MemoryFragment],
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<String> {
    let mut lines = Vec::new();
    for memory in memories.iter().take(3) {
        let content = if let Some(pos) = memory.content.find("\nI responded:") {
            &memory.content[..pos]
        } else {
            &memory.content
        };
        let Some(filtered) =
            crate::memory_retractions::filter_retracted_lines(content, retractions)
        else {
            continue;
        };
        let escaped = prompt_cap_chars(&filtered, 320)
            .replace("</memory-context>", "&lt;/memory-context&gt;");
        lines.push(format!("- {escaped}"));
    }
    if lines.is_empty() {
        None
    } else {
        Some(format!(
            "## Retrieved Memory Capsule\n<memory-context>\n{}\n</memory-context>",
            lines.join("\n")
        ))
    }
}

pub(crate) fn append_recalled_memory_context(
    system_prompt: &mut String,
    memories: &[MemoryFragment],
    retractions: &[crate::memory_retractions::MemoryRetraction],
    compact: bool,
) {
    if memories.is_empty() {
        return;
    }
    if compact {
        if let Some(section) = compact_recalled_memory_section(memories, retractions) {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&section);
        }
        return;
    }

    let mem_pairs: Vec<(String, String)> = memories
        .iter()
        .filter_map(|m| {
            let content = if let Some(pos) = m.content.find("\nI responded:") {
                m.content[..pos].to_string()
            } else {
                m.content.clone()
            };
            crate::memory_retractions::filter_retracted_lines(&content, retractions)
                .map(|content| (String::new(), content))
        })
        .collect();
    if !mem_pairs.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&crate::prompt_builder::build_memory_section(&mem_pairs));
    }
}

pub(crate) async fn append_runtime_context(
    system_prompt: &mut String,
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    user_message: &str,
    retractions: &[crate::memory_retractions::MemoryRetraction],
    compact: bool,
) {
    if compact {
        append_codex_runtime_context_capsule(
            system_prompt,
            manifest,
            kernel,
            user_message,
            retractions,
        )
        .await;
        return;
    }

    append_runtime_config_context(system_prompt, kernel).await;
    append_runtime_awareness_context(system_prompt, manifest, kernel, user_message, retractions);
}

pub(crate) async fn append_runtime_config_context(
    system_prompt: &mut String,
    kernel: Option<&Arc<dyn KernelHandle>>,
) {
    let now = chrono::Local::now();
    system_prompt.push_str(&format!(
        "\n\nCurrent time: {} ({})",
        now.format("%Y-%m-%d %H:%M %Z"),
        now.format("%A")
    ));

    if let Some(kh) = kernel {
        if let Some(ctx) = kh.get_channels_context().await {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&ctx);
        }
    }
    if let Some(tts_ctx) = build_tts_config_truth_section(kernel) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&tts_ctx);
    }
}

pub(crate) fn append_runtime_awareness_context(
    system_prompt: &mut String,
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    user_message: &str,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) {
    if let Some(kh) = kernel {
        let thoughts = kh.consume_thoughts(3);
        if !thoughts.is_empty() {
            system_prompt.push_str("\n\n[PENSEES EMERGENTES — ton graphe memoire a fait emerger ces connexions depuis la derniere interaction]\n");
            for t in &thoughts {
                let summary = t["summary"].as_str().unwrap_or("");
                let score = t["score"].as_f64().unwrap_or(0.0);
                system_prompt.push_str(&format!("- ({:.1}) {}\n", score, summary));
            }
            system_prompt.push_str(
                "Considere si ces elements sont pertinents pour la conversation en cours.\n",
            );
        }

        let reflections = kh.recall_reflections(&manifest.name, 5);
        if !reflections.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&reflections);
        }

        let user_ctx = kh.update_user_state(user_message);
        if !user_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&user_ctx);
        }

        system_prompt.push_str("\n\n");
        system_prompt.push_str(&french_datetime());

        let mood_ctx = kh.mood_prompt();
        if !mood_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&mood_ctx);
        }

        let operational_ctx = kh.operational_awareness_prompt(&manifest.name);
        if !operational_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&operational_ctx);
        }

        let temporal_ctx = kh.temporal_prompt();
        if !temporal_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&temporal_ctx);
        }

        let knowledge_ctx = kh.shared_knowledge_prompt();
        if let Some(knowledge_ctx) =
            crate::memory_retractions::filter_retracted_lines(&knowledge_ctx, retractions)
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&knowledge_ctx);
        }

        let curiosity_ctx = kh.curiosity_prompt();
        if !curiosity_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&curiosity_ctx);
        }

        let narration_ctx = kh.narration_prompt();
        if !narration_ctx.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&narration_ctx);
        }
    }
}

async fn append_codex_runtime_context_capsule(
    system_prompt: &mut String,
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    user_message: &str,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) {
    let mut capsule = String::new();
    let now = chrono::Local::now();
    capsule.push_str(&format!(
        "now: {} ({})",
        now.format("%Y-%m-%d %H:%M %Z"),
        now.format("%A")
    ));

    if let Some(kh) = kernel {
        if let Some(channels) = kh.get_channels_context().await {
            push_compact_section(&mut capsule, "Channels", &channels, 420);
        }
        if let Some(tts_ctx) = build_tts_config_truth_section(kernel) {
            push_compact_section(&mut capsule, "Runtime Config Truth", &tts_ctx, 360);
        }

        let thoughts = kh.consume_thoughts(1);
        if !thoughts.is_empty() {
            let thought_lines = thoughts
                .iter()
                .filter_map(|t| {
                    let summary = t["summary"].as_str()?;
                    let score = t["score"].as_f64().unwrap_or(0.0);
                    Some(format!("- ({score:.1}) {summary}"))
                })
                .collect::<Vec<_>>()
                .join("\n");
            push_compact_section(&mut capsule, "Emergent Thought", &thought_lines, 300);
        }

        push_compact_section(
            &mut capsule,
            "Past Reflections",
            &kh.recall_reflections(&manifest.name, 2),
            420,
        );
        push_compact_section(
            &mut capsule,
            "User State",
            &kh.update_user_state(user_message),
            280,
        );
        push_compact_section(&mut capsule, "Fresh Date", &french_datetime(), 220);
        push_compact_section(&mut capsule, "System Mood", &kh.mood_prompt(), 220);
        push_compact_section(
            &mut capsule,
            "Operational Awareness",
            &kh.operational_awareness_prompt(&manifest.name),
            420,
        );
        push_compact_section(
            &mut capsule,
            "Temporal Patterns",
            &kh.temporal_prompt(),
            260,
        );

        let knowledge_ctx = kh.shared_knowledge_prompt();
        if let Some(knowledge_ctx) =
            crate::memory_retractions::filter_retracted_lines(&knowledge_ctx, retractions)
        {
            push_compact_section(&mut capsule, "Shared Knowledge", &knowledge_ctx, 700);
        }
        push_compact_section(&mut capsule, "Curiosity", &kh.curiosity_prompt(), 240);
        push_compact_section(&mut capsule, "Narration", &kh.narration_prompt(), 220);
    }

    system_prompt.push_str("\n\n## Runtime Context Capsule\n");
    system_prompt.push_str(
        "Compact live state for this turn. Rehydrate exact details with tools if needed.\n",
    );
    system_prompt.push_str(&capsule);
}

pub(crate) fn build_tts_config_truth_section(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Option<String> {
    let kh = kernel?;
    let enabled = kh.config_read("tts.enabled").ok().flatten();
    let audio_provider = kh.config_read("media.audio_provider").ok().flatten();
    let native_voice_enabled = audio_provider.as_deref()
        == Some(crate::native_voice::WHISPER_PROVIDER)
        || kh.config_read("tts.provider").ok().flatten().as_deref()
            == Some(crate::native_voice::NATIVE_TTS_PROVIDER);
    if enabled.as_deref() != Some("true") && !native_voice_enabled {
        return None;
    }
    let provider = kh
        .config_read("tts.provider")
        .ok()
        .flatten()
        .unwrap_or_else(|| "auto".to_string());
    let voice_line = match provider.as_str() {
        "elevenlabs" => {
            let voice_id = kh
                .config_read("tts.elevenlabs.voice_id")
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".to_string());
            format!("TTS provider=elevenlabs, configured_voice_id={voice_id}.")
        }
        "openai" => {
            let voice = kh
                .config_read("tts.openai.voice")
                .ok()
                .flatten()
                .unwrap_or_else(|| "unknown".to_string());
            format!("TTS provider=openai, configured_voice={voice}.")
        }
        other => format!("TTS provider={other}."),
    };
    let native_voice = crate::native_voice::status();
    let native_line = if native_voice_enabled {
        format!(
            "\nNative voice: STT={} via local whisper.cpp small; TTS={} via {}; Telegram voice notes are transcribed automatically when audio is received. For explicit voice work, call speech_to_text/text_to_speech directly; do not search for a voice tool first.",
            if native_voice.stt_ready {
                "ready"
            } else {
                "pending"
            },
            if native_voice.tts_ready {
                "ready"
            } else {
                "pending"
            },
            native_voice.tts_engine.unwrap_or("local-native"),
        )
    } else {
        String::new()
    };
    Some(format!(
        "## Runtime Config Truth\n{voice_line}{native_line}\nconfig.toml is the source of truth for TTS provider and voice. Memories may mirror this setting, but must never override it; if memory conflicts, trust config_read/config.toml and update memory only after the config is changed."
    ))
}

/// Returns a fresh French-locale datetime string for injection at each agent turn.
pub(crate) fn french_datetime() -> String {
    let now = chrono::Local::now();
    let day = match now.format("%u").to_string().as_str() {
        "1" => "lundi",
        "2" => "mardi",
        "3" => "mercredi",
        "4" => "jeudi",
        "5" => "vendredi",
        "6" => "samedi",
        "7" => "dimanche",
        _ => "?",
    };
    format!(
        "[CURRENT TIME] {} {} {}h{} ({})\n",
        day,
        now.format("%d/%m/%Y"),
        now.format("%H"),
        now.format("%M"),
        now.format("%Z"),
    )
}

#[cfg(test)]
#[path = "agent_loop_context_tests.rs"]
mod tests;
