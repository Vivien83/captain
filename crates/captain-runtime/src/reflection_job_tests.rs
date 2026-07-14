use async_trait::async_trait;
use std::sync::Mutex;
use std::time::Duration;

use crate::learning_bus::LearningSignal;
use crate::outcome_detector::{ClassifiedSignal, Outcome};
use crate::reflection_job::{
    build_prompt, parse_candidates, run_reflection, spawn_consumer, LlmDriverCompleter,
    NoopCompleter, ReflectionCompleter, ReflectionConfig,
};

fn classified(outcome: Outcome, signal: LearningSignal) -> ClassifiedSignal {
    ClassifiedSignal { outcome, signal }
}

fn explicit_remember_sig() -> LearningSignal {
    LearningSignal::ExplicitRemember {
        agent_id: "captain".into(),
        user_msg: "retiens que j'aime le café sans sucre".into(),
        source: "ws".into(),
    }
}

fn cfg() -> ReflectionConfig {
    ReflectionConfig {
        primary_model: "model-a".into(),
        fallback_models: vec!["model-b".into(), "model-c".into()],
        timeout_secs: 2,
        min_confidence: 0.7,
    }
}

struct EchoCompleter {
    response: String,
}

#[async_trait]
impl ReflectionCompleter for EchoCompleter {
    async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
        Ok(self.response.clone())
    }
}

struct FailCompleter;

#[async_trait]
impl ReflectionCompleter for FailCompleter {
    async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
        Err("boom".into())
    }
}

struct SlowCompleter;

#[async_trait]
impl ReflectionCompleter for SlowCompleter {
    async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Ok("[]".into())
    }
}

struct FlakyCompleter {
    fails_left: Mutex<i32>,
    good_response: String,
}

#[async_trait]
impl ReflectionCompleter for FlakyCompleter {
    async fn complete(&self, _m: &str, _s: &str, _u: &str) -> Result<String, String> {
        let mut n = self.fails_left.lock().unwrap();
        if *n > 0 {
            *n -= 1;
            Err("transient".into())
        } else {
            Ok(self.good_response.clone())
        }
    }
}

#[test]
fn prompt_includes_given_when_then_sections() {
    let cs = classified(Outcome::ExplicitRemember, explicit_remember_sig());
    let (_sys, user) = build_prompt(&cs);
    assert!(user.contains("GIVEN:"));
    assert!(user.contains("WHEN:"));
    assert!(user.contains("THEN WHAT TO REMEMBER:"));
    assert!(user.contains("captain"));
    assert!(user.contains("café"));
}

#[test]
fn prompt_system_forbids_secrets_and_requires_json_only() {
    let cs = classified(Outcome::Success, explicit_remember_sig());
    let (sys, _user) = build_prompt(&cs);
    assert!(sys.to_lowercase().contains("json only") || sys.contains("ONLY a JSON array"));
    assert!(sys.to_lowercase().contains("secret"));
}

#[test]
fn parse_happy_path() {
    let raw = r#"[
        {"wing":"learnings","room":"general","subject":"user","predicate":"likes","object":"coffee","confidence":0.9}
    ]"#;
    let c = parse_candidates(raw);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].subject, "user");
    assert_eq!(c[0].confidence, 0.9);
}

#[test]
fn parse_tolerates_prose_around_json() {
    let raw = "Sure:\n[{\"wing\":\"learnings\",\"room\":\"general\",\"subject\":\"x\",\"predicate\":\"y\",\"object\":\"z\",\"confidence\":0.8}]\nDone";
    let c = parse_candidates(raw);
    assert_eq!(c.len(), 1);
}

#[test]
fn parse_empty_array_returns_empty_vec() {
    assert!(parse_candidates("[]").is_empty());
}

#[test]
fn parse_not_json_returns_empty_vec() {
    assert!(parse_candidates("I don't think there's anything to remember.").is_empty());
}

#[test]
fn parse_skips_malformed_entries() {
    let raw = r#"[
        {"wing":"learnings","room":"general","subject":"a","predicate":"b","object":"c","confidence":0.9},
        {"wing":"learnings","room":"general","subject":"d"},
        {"wing":"learnings","room":"general","subject":"e","predicate":"f","object":"g","confidence":0.9}
    ]"#;
    let c = parse_candidates(raw);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].subject, "a");
    assert_eq!(c[1].subject, "e");
}

#[tokio::test]
async fn run_returns_filtered_candidates_on_success() {
    let c = EchoCompleter {
        response: r#"[
            {"wing":"learnings","room":"general","subject":"a","predicate":"b","object":"c","confidence":0.9},
            {"wing":"learnings","room":"general","subject":"low","predicate":"p","object":"o","confidence":0.3}
        ]"#
        .into(),
    };
    let out = run_reflection(
        &c,
        &cfg(),
        &classified(Outcome::ExplicitRemember, explicit_remember_sig()),
    )
    .await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].subject, "a");
}

#[tokio::test]
async fn run_falls_back_through_chain() {
    let c = FlakyCompleter {
        fails_left: Mutex::new(2),
        good_response:
            r#"[{"wing":"w","room":"r","subject":"s","predicate":"p","object":"o","confidence":0.9}]"#
                .into(),
    };
    let out = run_reflection(
        &c,
        &cfg(),
        &classified(Outcome::UserCorrected, explicit_remember_sig()),
    )
    .await;
    assert_eq!(out.len(), 1);
    assert_eq!(*c.fails_left.lock().unwrap(), 0);
}

#[tokio::test]
async fn run_returns_empty_when_all_fail() {
    let out = run_reflection(
        &FailCompleter,
        &cfg(),
        &classified(Outcome::Failure, explicit_remember_sig()),
    )
    .await;
    assert!(out.is_empty());
}

#[tokio::test]
async fn run_timeout_returns_empty() {
    let out = run_reflection(
        &SlowCompleter,
        &ReflectionConfig {
            primary_model: "a".into(),
            fallback_models: vec![],
            timeout_secs: 1,
            min_confidence: 0.7,
        },
        &classified(Outcome::UserCorrected, explicit_remember_sig()),
    )
    .await;
    assert!(out.is_empty());
}

#[tokio::test]
async fn noop_completer_returns_empty_candidates() {
    let out = run_reflection(
        &NoopCompleter,
        &cfg(),
        &classified(Outcome::ExplicitRemember, explicit_remember_sig()),
    )
    .await;
    assert!(out.is_empty());
}

#[tokio::test]
async fn consumer_forwards_nonempty_batches() {
    let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
    let completer: std::sync::Arc<dyn ReflectionCompleter> =
        std::sync::Arc::new(EchoCompleter {
            response: r#"[{"wing":"w","room":"r","subject":"s","predicate":"p","object":"o","confidence":0.9}]"#.into(),
        });
    let (_h, mut out_rx) = spawn_consumer(in_rx, completer, cfg(), 4);

    in_tx
        .send(classified(
            Outcome::ExplicitRemember,
            explicit_remember_sig(),
        ))
        .await
        .unwrap();
    drop(in_tx);

    let batch = out_rx.recv().await.unwrap();
    assert_eq!(batch.candidates.len(), 1);
    assert_eq!(batch.outcome, Outcome::ExplicitRemember);
    assert_eq!(batch.agent_id, "captain");
}

#[tokio::test]
async fn consumer_drops_empty_batches() {
    let (in_tx, in_rx) = tokio::sync::mpsc::channel(4);
    let completer: std::sync::Arc<dyn ReflectionCompleter> = std::sync::Arc::new(NoopCompleter);
    let (_h, mut out_rx) = spawn_consumer(in_rx, completer, cfg(), 4);

    in_tx
        .send(classified(Outcome::Success, explicit_remember_sig()))
        .await
        .unwrap();
    drop(in_tx);

    let res = tokio::time::timeout(Duration::from_millis(50), out_rx.recv()).await;
    assert!(res.is_err() || res.unwrap().is_none());
}

#[tokio::test]
async fn llm_driver_completer_wraps_driver_and_extracts_text() {
    use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
    use captain_types::message::{ContentBlock, StopReason, TokenUsage};

    struct JsonDriver;

    #[async_trait]
    impl LlmDriver for JsonDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: r#"[{"wing":"learnings","room":"user_preferences","subject":"user","predicate":"prefers","object":"dark mode","confidence":0.95}]"#.to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    ..Default::default()
                },
            })
        }
    }

    let completer = LlmDriverCompleter::new(std::sync::Arc::new(JsonDriver));
    let raw = completer
        .complete("model-a", "system", "user")
        .await
        .unwrap();
    let parsed = parse_candidates(&raw);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].subject, "user");
}

#[test]
fn reflection_config_from_learning_config() {
    let lc = captain_types::config::LearningConfig::default();
    let rc: ReflectionConfig = (&lc).into();
    assert_eq!(rc.primary_model, "gpt-5.5");
    assert!(rc.fallback_models.is_empty());
    assert_eq!(rc.timeout_secs, 30);
    assert!(rc.min_confidence < f32::EPSILON);
}
