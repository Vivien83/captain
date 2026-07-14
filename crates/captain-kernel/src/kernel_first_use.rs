use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use captain_runtime::agent_loop::AgentLoopResult;
use captain_types::agent::AgentEntry;
use captain_types::error::CaptainError;
use tracing::warn;

use super::kernel_first_use_text::{
    first_use_clean_answer, first_use_completed_response, first_use_intro, first_use_locale,
    first_use_next_prompt, first_use_skip_requested, first_use_trivial_greeting,
    read_global_user_profile, FIRST_USE_ONBOARDING_QUESTIONS, FIRST_USE_ONBOARDING_STATE_FILE,
    GLOBAL_USER_PROFILE_END, GLOBAL_USER_PROFILE_START,
};
use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FirstUseOnboardingState {
    step: usize,
    answers: BTreeMap<String, String>,
    pending_request: Option<String>,
    locale: String,
    started_at: String,
    updated_at: String,
}

struct FirstUseConfigDraft {
    path: PathBuf,
    old_size: usize,
    old_top_keys: BTreeSet<String>,
    doc: toml_edit::DocumentMut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FirstUseVoicePreference {
    ElevenLabs,
    OpenAi(&'static str),
    Disabled,
    Unchanged,
}

fn first_use_answer<'a>(state: &'a FirstUseOnboardingState, key: &str) -> Option<&'a str> {
    state
        .answers
        .get(key)
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

fn first_use_initial_pending_request(user_message: &str) -> Option<String> {
    if first_use_trivial_greeting(user_message) {
        return None;
    }
    Some(first_use_clean_answer(user_message))
        .filter(|s| !s.trim().is_empty() && !first_use_skip_requested(s))
}

fn classify_first_use_voice_preference(voice: &str) -> FirstUseVoicePreference {
    let lower = voice.to_ascii_lowercase();
    if lower.contains("eleven") {
        return FirstUseVoicePreference::ElevenLabs;
    }
    if lower.contains("openai")
        || lower.contains("nova")
        || lower.contains("alloy")
        || lower.contains("echo")
        || lower.contains("fable")
        || lower.contains("onyx")
        || lower.contains("shimmer")
    {
        let openai_voice = ["nova", "alloy", "echo", "fable", "onyx", "shimmer"]
            .into_iter()
            .find(|candidate| lower.contains(candidate))
            .unwrap_or("nova");
        return FirstUseVoicePreference::OpenAi(openai_voice);
    }
    if lower.contains("pas de voix")
        || lower.contains("aucune")
        || lower.contains("no voice")
        || lower.contains("none")
        || lower == "non"
        || lower == "no"
    {
        return FirstUseVoicePreference::Disabled;
    }
    FirstUseVoicePreference::Unchanged
}

fn set_onboarding_toml_value(
    doc: &mut toml_edit::DocumentMut,
    path: &[&str],
    key: &str,
    value: toml_edit::Item,
) -> KernelResult<()> {
    let mut cursor: &mut toml_edit::Item = doc.as_item_mut();
    for segment in path {
        if !cursor.is_table_like() {
            *cursor = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = cursor.as_table_mut().ok_or_else(|| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Config path '{}' is not a table",
                path.join(".")
            )))
        })?;
        if !table.contains_key(segment) {
            let mut new_table = toml_edit::Table::new();
            new_table.set_implicit(false);
            table.insert(segment, toml_edit::Item::Table(new_table));
        }
        cursor = table.get_mut(segment).ok_or_else(|| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to create config path '{}'",
                path.join(".")
            )))
        })?;
    }

    let table = cursor.as_table_mut().ok_or_else(|| {
        KernelError::Captain(CaptainError::Internal(format!(
            "Config parent '{}' is not a table",
            path.join(".")
        )))
    })?;
    table.insert(key, value);
    Ok(())
}

fn onboarding_config_top_keys(doc: &toml_edit::DocumentMut) -> BTreeSet<String> {
    doc.as_table().iter().map(|(k, _)| k.to_string()).collect()
}

fn validate_onboarding_config_write(
    old_size: usize,
    old_top_keys: &BTreeSet<String>,
    doc: &toml_edit::DocumentMut,
    serialized: &str,
) -> KernelResult<()> {
    let new_top_keys = onboarding_config_top_keys(doc);
    let lost: Vec<&String> = old_top_keys.difference(&new_top_keys).collect();
    if !lost.is_empty() {
        return Err(KernelError::Captain(CaptainError::Internal(format!(
            "Refusing onboarding config write: top-level keys would be lost: {lost:?}"
        ))));
    }

    if old_size > 100 && serialized.len() < (old_size * 7 / 10) {
        return Err(KernelError::Captain(CaptainError::Internal(format!(
            "Refusing onboarding config write: suspicious shrinkage ({} -> {} bytes)",
            old_size,
            serialized.len()
        ))));
    }
    Ok(())
}

fn write_onboarding_config_atomic(config_path: &Path, serialized: String) -> KernelResult<()> {
    let tmp_path = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, serialized).map_err(|e| {
        KernelError::Captain(CaptainError::Internal(format!(
            "Failed to write onboarding config tmp: {e}"
        )))
    })?;
    std::fs::rename(&tmp_path, config_path).map_err(|e| {
        KernelError::Captain(CaptainError::Internal(format!(
            "Failed to replace config.toml after onboarding: {e}"
        )))
    })
}

impl CaptainKernel {
    fn first_use_onboarding_state_path(&self) -> PathBuf {
        self.config.home_dir.join(FIRST_USE_ONBOARDING_STATE_FILE)
    }

    fn should_run_first_use_onboarding(
        &self,
        entry: &AgentEntry,
        channel_type: Option<&str>,
    ) -> bool {
        let is_subagent = entry
            .manifest
            .metadata
            .get("is_subagent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_subagent {
            return false;
        }

        let is_principal =
            entry.name == "captain" || entry.manifest.name.eq_ignore_ascii_case("captain");
        if !is_principal {
            return false;
        }

        !matches!(
            channel_type,
            Some("cron" | "workflow" | "background" | "system" | "agent")
        )
    }

    fn load_first_use_onboarding_state(&self) -> Option<FirstUseOnboardingState> {
        let path = self.first_use_onboarding_state_path();
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn save_first_use_onboarding_state(&self, state: &FirstUseOnboardingState) -> KernelResult<()> {
        std::fs::create_dir_all(&self.config.home_dir).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to create Captain home for onboarding: {e}"
            )))
        })?;
        let path = self.first_use_onboarding_state_path();
        let serialized = serde_json::to_string_pretty(state).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to serialize onboarding state: {e}"
            )))
        })?;
        std::fs::write(&path, serialized).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to write onboarding state {}: {e}",
                path.display()
            )))
        })
    }

    fn clear_first_use_onboarding_state(&self) {
        let path = self.first_use_onboarding_state_path();
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Failed to remove onboarding state")
            }
        }
    }

    fn write_global_user_profile_block(
        &self,
        state: &FirstUseOnboardingState,
        skipped: bool,
    ) -> KernelResult<()> {
        std::fs::create_dir_all(&self.config.home_dir).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to create Captain home for USER.md: {e}"
            )))
        })?;

        let preferred_name = first_use_answer(state, "preferred_name").unwrap_or("(not set)");
        let language = first_use_answer(state, "language").unwrap_or(&self.config.language);
        let timezone = first_use_answer(state, "timezone").unwrap_or(&self.config.timezone);
        let answer_style =
            first_use_answer(state, "answer_style").unwrap_or(&self.config.assistant.style);
        let voice_preference = first_use_answer(state, "voice_preference").unwrap_or("(not set)");
        let notifications = first_use_answer(state, "notifications").unwrap_or("(not set)");
        let privacy = first_use_answer(state, "privacy").unwrap_or("(not set)");
        let pending = state.pending_request.as_deref().unwrap_or("(none)");

        let block = format!(
            "{GLOBAL_USER_PROFILE_START}\n\
             # User profile\n\
             - Preferred name: {preferred_name}\n\
             - Preferred language: {language}\n\
             - Timezone: {timezone}\n\
             - Preferred answer style: {answer_style}\n\
             - Voice preference: {voice_preference}\n\
             - Notification preference: {notifications}\n\
             - Privacy boundaries: {privacy}\n\
             - First interview status: {}\n\
             - First pending request: {pending}\n\
             {GLOBAL_USER_PROFILE_END}\n",
            if skipped { "skipped" } else { "completed" }
        );

        let path = self.config.home_dir.join("USER.md");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let updated = if let (Some(start), Some(end)) = (
            existing.find(GLOBAL_USER_PROFILE_START),
            existing.find(GLOBAL_USER_PROFILE_END),
        ) {
            let end_idx = end + GLOBAL_USER_PROFILE_END.len();
            format!(
                "{}{}{}",
                &existing[..start],
                block,
                existing[end_idx..].trim_start_matches('\n')
            )
        } else if existing.trim().is_empty() {
            block
        } else {
            format!("{}\n\n{}", existing.trim_end(), block)
        };

        std::fs::write(&path, updated).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to write global USER.md {}: {e}",
                path.display()
            )))
        })
    }

    fn patch_first_use_onboarding_config(
        &self,
        state: &FirstUseOnboardingState,
    ) -> KernelResult<()> {
        let mut draft = self.load_first_use_config_draft()?;
        self.apply_first_use_config_answers(&mut draft.doc, state)?;
        self.apply_first_use_voice_preference(&mut draft.doc, state)?;
        let serialized = draft.doc.to_string();
        validate_onboarding_config_write(
            draft.old_size,
            &draft.old_top_keys,
            &draft.doc,
            &serialized,
        )?;
        write_onboarding_config_atomic(&draft.path, serialized)
    }

    fn load_first_use_config_draft(&self) -> KernelResult<FirstUseConfigDraft> {
        std::fs::create_dir_all(&self.config.home_dir).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to create Captain home for config.toml: {e}"
            )))
        })?;
        let config_path = self.config.home_dir.join("config.toml");
        let raw = if config_path.exists() {
            std::fs::read_to_string(&config_path).map_err(|e| {
                KernelError::Captain(CaptainError::Internal(format!(
                    "Failed to read config.toml for onboarding: {e}"
                )))
            })?
        } else {
            String::new()
        };
        let old_size = raw.len();
        let doc: toml_edit::DocumentMut = raw.parse().map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to parse config.toml for onboarding: {e}"
            )))
        })?;
        let old_top_keys = onboarding_config_top_keys(&doc);
        Ok(FirstUseConfigDraft {
            path: config_path,
            old_size,
            old_top_keys,
            doc,
        })
    }

    fn apply_first_use_config_answers(
        &self,
        doc: &mut toml_edit::DocumentMut,
        state: &FirstUseOnboardingState,
    ) -> KernelResult<()> {
        set_onboarding_toml_value(
            doc,
            &["assistant"],
            "onboarding_completed",
            toml_edit::value(true),
        )?;

        if let Some(language) = first_use_answer(state, "language") {
            set_onboarding_toml_value(doc, &[], "language", toml_edit::value(language))?;
        }
        if let Some(timezone) = first_use_answer(state, "timezone") {
            set_onboarding_toml_value(doc, &[], "timezone", toml_edit::value(timezone))?;
        }
        if let Some(style) = first_use_answer(state, "answer_style") {
            set_onboarding_toml_value(doc, &["assistant"], "style", toml_edit::value(style))?;
        }
        Ok(())
    }

    fn apply_first_use_voice_preference(
        &self,
        doc: &mut toml_edit::DocumentMut,
        state: &FirstUseOnboardingState,
    ) -> KernelResult<()> {
        let Some(voice) = first_use_answer(state, "voice_preference") else {
            return Ok(());
        };
        match classify_first_use_voice_preference(voice) {
            FirstUseVoicePreference::ElevenLabs => self.apply_first_use_elevenlabs_voice(doc),
            FirstUseVoicePreference::OpenAi(openai_voice) => {
                self.apply_first_use_openai_voice(doc, openai_voice)
            }
            FirstUseVoicePreference::Disabled => {
                set_onboarding_toml_value(doc, &["tts"], "enabled", toml_edit::value(false))
            }
            FirstUseVoicePreference::Unchanged => Ok(()),
        }
    }

    fn apply_first_use_elevenlabs_voice(
        &self,
        doc: &mut toml_edit::DocumentMut,
    ) -> KernelResult<()> {
        set_onboarding_toml_value(doc, &["tts"], "provider", toml_edit::value("elevenlabs"))?;
        set_onboarding_toml_value(
            doc,
            &["tts"],
            "enabled",
            toml_edit::value(self.first_use_elevenlabs_ready()),
        )
    }

    fn apply_first_use_openai_voice(
        &self,
        doc: &mut toml_edit::DocumentMut,
        openai_voice: &'static str,
    ) -> KernelResult<()> {
        set_onboarding_toml_value(doc, &["tts"], "provider", toml_edit::value("openai"))?;
        set_onboarding_toml_value(
            doc,
            &["tts"],
            "enabled",
            toml_edit::value(self.first_use_openai_tts_ready()),
        )?;
        set_onboarding_toml_value(
            doc,
            &["tts", "openai"],
            "voice",
            toml_edit::value(openai_voice),
        )
    }

    fn first_use_elevenlabs_ready(&self) -> bool {
        std::env::var("ELEVENLABS_API_KEY")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            || (self.config.tts.enabled
                && self.config.tts.provider.as_deref() == Some("elevenlabs"))
    }

    fn first_use_openai_tts_ready(&self) -> bool {
        std::env::var("OPENAI_API_KEY")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
            || (self.config.tts.enabled && self.config.tts.provider.as_deref() == Some("openai"))
    }

    fn ensure_global_user_profile_from_config(&self) {
        if read_global_user_profile(&self.config.home_dir).is_some() {
            return;
        }
        let mut answers = BTreeMap::new();
        answers.insert("language".to_string(), self.config.language.clone());
        answers.insert("timezone".to_string(), self.config.timezone.clone());
        answers.insert(
            "answer_style".to_string(),
            self.config.assistant.style.clone(),
        );
        let voice = match self.config.tts.provider.as_deref() {
            Some("elevenlabs") => format!(
                "ElevenLabs ({})",
                self.config.tts.elevenlabs.voice_id.as_str()
            ),
            Some("openai") => format!("OpenAI {}", self.config.tts.openai.voice.as_str()),
            Some(provider) => provider.to_string(),
            None => "(not set)".to_string(),
        };
        answers.insert("voice_preference".to_string(), voice);
        let now = chrono::Utc::now().to_rfc3339();
        let state = FirstUseOnboardingState {
            step: FIRST_USE_ONBOARDING_QUESTIONS.len(),
            answers,
            pending_request: None,
            locale: first_use_locale(&self.config.language, "").to_string(),
            started_at: now.clone(),
            updated_at: now,
        };
        if let Err(e) = self.write_global_user_profile_block(&state, true) {
            warn!(error = %e, "Failed to backfill global USER.md from config");
        }
    }

    fn complete_first_use_onboarding(
        &self,
        state: &FirstUseOnboardingState,
        skipped: bool,
    ) -> KernelResult<AgentLoopResult> {
        self.write_global_user_profile_block(state, skipped)?;
        self.patch_first_use_onboarding_config(state)?;
        self.clear_first_use_onboarding_state();
        Ok(Self::empty_agent_loop_result(first_use_completed_response(
            &state.locale,
            skipped,
            state.pending_request.as_deref(),
        )))
    }

    pub(super) fn maybe_handle_first_use_onboarding(
        &self,
        entry: &AgentEntry,
        user_message: &str,
        channel_type: Option<&str>,
    ) -> KernelResult<Option<AgentLoopResult>> {
        if !self.should_run_first_use_onboarding(entry, channel_type) {
            return Ok(None);
        }

        if self.first_use_onboarding_already_resolved() {
            return Ok(None);
        }

        let now = chrono::Utc::now().to_rfc3339();
        match self.load_first_use_onboarding_state() {
            Some(state) => self.continue_first_use_onboarding(state, user_message, now),
            None => self.start_first_use_onboarding(user_message, now),
        }
    }

    fn first_use_onboarding_already_resolved(&self) -> bool {
        if read_global_user_profile(&self.config.home_dir).is_some() {
            self.clear_first_use_onboarding_state();
            return true;
        }
        if self.config.assistant.onboarding_completed {
            self.ensure_global_user_profile_from_config();
            self.clear_first_use_onboarding_state();
            return true;
        }
        false
    }

    fn start_first_use_onboarding(
        &self,
        user_message: &str,
        now: String,
    ) -> KernelResult<Option<AgentLoopResult>> {
        let pending_request = first_use_initial_pending_request(user_message);
        let state = FirstUseOnboardingState {
            step: 0,
            answers: BTreeMap::new(),
            pending_request: pending_request.clone(),
            locale: first_use_locale(&self.config.language, user_message).to_string(),
            started_at: now.clone(),
            updated_at: now,
        };
        if first_use_skip_requested(user_message) {
            return Ok(Some(self.complete_first_use_onboarding(&state, true)?));
        }
        self.save_first_use_onboarding_state(&state)?;
        Ok(Some(Self::empty_agent_loop_result(first_use_intro(
            &state.locale,
            pending_request.as_deref(),
        ))))
    }

    fn continue_first_use_onboarding(
        &self,
        mut state: FirstUseOnboardingState,
        user_message: &str,
        now: String,
    ) -> KernelResult<Option<AgentLoopResult>> {
        if first_use_skip_requested(user_message) {
            state.updated_at = now;
            return Ok(Some(self.complete_first_use_onboarding(&state, true)?));
        }

        let answer = first_use_clean_answer(user_message);
        if answer.is_empty() {
            return Ok(Some(self.first_use_next_question_result(&state)));
        }

        if let Some((key, _, _)) = FIRST_USE_ONBOARDING_QUESTIONS.get(state.step) {
            state.answers.insert((*key).to_string(), answer);
        }
        state.step = state.step.saturating_add(1);
        state.updated_at = now;

        if state.step >= FIRST_USE_ONBOARDING_QUESTIONS.len() {
            return Ok(Some(self.complete_first_use_onboarding(&state, false)?));
        }

        self.save_first_use_onboarding_state(&state)?;
        Ok(Some(Self::empty_agent_loop_result(first_use_next_prompt(
            &state.locale,
            state.step,
        ))))
    }

    fn first_use_next_question_result(&self, state: &FirstUseOnboardingState) -> AgentLoopResult {
        Self::empty_agent_loop_result(first_use_next_prompt(
            &state.locale,
            state
                .step
                .min(FIRST_USE_ONBOARDING_QUESTIONS.len().saturating_sub(1)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_use_answer_trims_and_ignores_empty_values() {
        let mut state = test_state();
        state
            .answers
            .insert("language".to_string(), "  francais  ".to_string());
        state
            .answers
            .insert("timezone".to_string(), "   ".to_string());

        assert_eq!(first_use_answer(&state, "language"), Some("francais"));
        assert_eq!(first_use_answer(&state, "timezone"), None);
        assert_eq!(first_use_answer(&state, "missing"), None);
    }

    #[test]
    fn first_use_initial_pending_request_filters_greetings_and_skips() {
        assert_eq!(first_use_initial_pending_request("bonjour"), None);
        assert_eq!(first_use_initial_pending_request("/skip_onboarding"), None);
        assert_eq!(
            first_use_initial_pending_request("Peux-tu analyser mes mails ?"),
            Some("Peux-tu analyser mes mails ?".to_string())
        );
    }

    #[test]
    fn first_use_voice_preference_classifies_supported_choices() {
        assert_eq!(
            classify_first_use_voice_preference("OpenAI fable"),
            FirstUseVoicePreference::OpenAi("fable")
        );
        assert_eq!(
            classify_first_use_voice_preference("ElevenLabs voix rapide"),
            FirstUseVoicePreference::ElevenLabs
        );
        assert_eq!(
            classify_first_use_voice_preference("no voice"),
            FirstUseVoicePreference::Disabled
        );
        assert_eq!(
            classify_first_use_voice_preference("surprise me"),
            FirstUseVoicePreference::Unchanged
        );
    }

    #[test]
    fn onboarding_config_validation_rejects_lost_top_level_keys() {
        let old_doc: toml_edit::DocumentMut = r#"
language = "fr"

[assistant]
style = "court"
"#
        .parse()
        .unwrap();
        let new_doc: toml_edit::DocumentMut = r#"
[assistant]
style = "court"
"#
        .parse()
        .unwrap();
        let old_keys = onboarding_config_top_keys(&old_doc);

        assert!(validate_onboarding_config_write(
            old_doc.to_string().len(),
            &old_keys,
            &new_doc,
            &new_doc.to_string()
        )
        .is_err());
    }

    fn test_state() -> FirstUseOnboardingState {
        FirstUseOnboardingState {
            step: 0,
            answers: BTreeMap::new(),
            pending_request: None,
            locale: "fr".to_string(),
            started_at: "2026-06-20T00:00:00Z".to_string(),
            updated_at: "2026-06-20T00:00:00Z".to_string(),
        }
    }
}
