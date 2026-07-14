//! Video analysis runtime handler.

use std::path::{Path, PathBuf};

use crate::media_understanding::MediaEngine;

use super::{emit_progress, validate_path, ToolProgressEvent};

struct VideoAnalyzeOptions {
    fps: f32,
    max_frames: usize,
    prompt: Option<String>,
    transcribe: bool,
}

pub(crate) async fn tool_video_analyze(
    input: &serde_json::Value,
    media_engine: Option<&MediaEngine>,
    workspace_root: Option<&Path>,
    tool_use_id: &str,
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;
    let video_path = Path::new(path);
    if !video_path.exists() {
        return Err(format!("video file not found: {path}"));
    }

    let base = workspace_root
        .map(|p| p.join(".captain-video-cache"))
        .unwrap_or_else(|| std::env::temp_dir().join("captain-video-cache"));
    let work_dir = base.join(uuid::Uuid::new_v4().to_string());
    let frames_dir = work_dir.join("frames");
    let result = tool_video_analyze_inner(
        input,
        engine,
        path,
        video_path,
        &work_dir,
        &frames_dir,
        tool_use_id,
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;
    result
}

async fn tool_video_analyze_inner(
    input: &serde_json::Value,
    engine: &MediaEngine,
    path: &str,
    video_path: &Path,
    work_dir: &Path,
    frames_dir: &Path,
    tool_use_id: &str,
) -> Result<String, String> {
    let options = parse_video_analyze_options(input);
    crate::video::ensure_ffmpeg().await?;
    let (frames, audio_extract_res, audio_path) =
        extract_video_inputs(video_path, work_dir, frames_dir, &options).await?;
    if frames.is_empty() {
        return Err("ffmpeg produced 0 frames — is the input a valid video?".to_string());
    }

    downscale_video_frames(&frames).await?;
    let attachments = build_video_frame_attachments(&frames, &options).await?;
    let frames_count = attachments.len();
    let results = describe_video_frames(engine, attachments, tool_use_id, frames_count).await;
    let timeline = video_timeline_from_results(results, options.fps);
    let audio = video_audio_response(engine, &options, audio_extract_res, &audio_path).await;

    render_video_analyze_result(path, &options, frames_count, timeline, audio)
}

fn parse_video_analyze_options(input: &serde_json::Value) -> VideoAnalyzeOptions {
    VideoAnalyzeOptions {
        fps: input["fps"].as_f64().unwrap_or(1.0) as f32,
        max_frames: (input["max_frames"].as_u64().unwrap_or(10) as usize).min(60),
        prompt: input["prompt"].as_str().map(ToString::to_string),
        transcribe: input["transcribe"].as_bool().unwrap_or(false),
    }
}

async fn extract_video_inputs(
    video_path: &Path,
    work_dir: &Path,
    frames_dir: &Path,
    options: &VideoAnalyzeOptions,
) -> Result<(Vec<PathBuf>, Result<(), String>, PathBuf), String> {
    let cfg = crate::video::VideoFrameExtractConfig {
        fps: options.fps,
        max_frames: options.max_frames,
        output_dir: frames_dir.to_path_buf(),
    };
    let audio_path = work_dir.join("audio.mp3");
    let (frames_res, audio_extract_res) = if options.transcribe {
        tokio::join!(
            crate::video::extract_frames(video_path, &cfg),
            crate::video::extract_audio(video_path, &audio_path)
        )
    } else {
        (crate::video::extract_frames(video_path, &cfg).await, Ok(()))
    };
    Ok((frames_res?, audio_extract_res, audio_path))
}

async fn downscale_video_frames(frames: &[PathBuf]) -> Result<(), String> {
    let mut downscale_jobs: tokio::task::JoinSet<Result<(), String>> = tokio::task::JoinSet::new();
    for frame in frames {
        downscale_jobs.spawn(crate::video::downscale_to_long_edge_async(
            frame.clone(),
            crate::video::VISION_MAX_LONG_EDGE,
        ));
    }
    while let Some(joined) = downscale_jobs.join_next().await {
        joined.map_err(|e| format!("downscale join error: {e}"))??;
    }
    Ok(())
}

async fn build_video_frame_attachments(
    frames: &[PathBuf],
    options: &VideoAnalyzeOptions,
) -> Result<Vec<captain_types::media::MediaAttachment>, String> {
    use base64::Engine;
    use captain_types::media::{MediaAttachment, MediaSource, MediaType};

    let frames_count = frames.len();
    let mut attachments = Vec::with_capacity(frames_count);
    for (i, frame) in frames.iter().enumerate() {
        let bytes = tokio::fs::read(frame)
            .await
            .map_err(|e| format!("Failed to read frame {}: {e}", frame.display()))?;
        attachments.push(MediaAttachment {
            media_type: MediaType::Image,
            mime_type: "image/jpeg".to_string(),
            source: MediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                mime_type: "image/jpeg".to_string(),
            },
            size_bytes: bytes.len() as u64,
            context_hint: frame_context_hint(options, i, frames_count),
            batch_size_hint: Some(frames_count),
        });
    }
    Ok(attachments)
}

fn frame_context_hint(
    options: &VideoAnalyzeOptions,
    frame_index: usize,
    frames_count: usize,
) -> Option<String> {
    let temporal = format!(
        "Frame {}/{} d'une vidéo, t = {:.1}s.",
        frame_index + 1,
        frames_count,
        frame_timestamp_seconds(frame_index, options.fps)
    );
    match options
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(user_hint) => Some(format!("{user_hint}\n{temporal}")),
        None => Some(temporal),
    }
}

fn frame_timestamp_seconds(frame_index: usize, fps: f32) -> f32 {
    if fps > 0.0 {
        (frame_index as f32) / fps
    } else {
        0.0
    }
}

async fn describe_video_frames(
    engine: &MediaEngine,
    attachments: Vec<captain_types::media::MediaAttachment>,
    tool_use_id: &str,
    frames_count: usize,
) -> Vec<Option<Result<captain_types::media::MediaUnderstanding, String>>> {
    let mut describe_jobs: tokio::task::JoinSet<(
        usize,
        Result<captain_types::media::MediaUnderstanding, String>,
    )> = tokio::task::JoinSet::new();
    for (i, attachment) in attachments.into_iter().enumerate() {
        let engine = engine.clone();
        describe_jobs.spawn(async move { (i, engine.describe_image(&attachment).await) });
    }
    let mut results: Vec<Option<Result<captain_types::media::MediaUnderstanding, String>>> =
        (0..frames_count).map(|_| None).collect();
    let mut completed = 0usize;
    while let Some(joined) = describe_jobs.join_next().await {
        let (idx, res) = match joined {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "frame describe task panicked");
                continue;
            }
        };
        results[idx] = Some(res);
        completed += 1;
        emit_progress(ToolProgressEvent {
            tool_use_id: tool_use_id.to_string(),
            message: format!("Frame {completed}/{frames_count} décrite"),
            frame_index: Some(idx),
            frames_total: Some(frames_count),
        });
    }
    results
}

fn video_timeline_from_results(
    results: Vec<Option<Result<captain_types::media::MediaUnderstanding, String>>>,
    fps: f32,
) -> Vec<serde_json::Value> {
    let mut timeline = Vec::with_capacity(results.len());
    for (i, slot) in results.into_iter().enumerate() {
        let mut entry = serde_json::json!({
            "index": i,
            "t_seconds": frame_timestamp_seconds(i, fps),
        });
        match slot {
            Some(Ok(u)) => {
                entry["description"] = serde_json::json!(u.description);
                entry["provider"] = serde_json::json!(u.provider);
                entry["model"] = serde_json::json!(u.model);
            }
            Some(Err(e)) => entry["error"] = serde_json::json!(e),
            None => entry["error"] = serde_json::json!("frame task panicked"),
        }
        timeline.push(entry);
    }
    timeline
}

async fn video_audio_response(
    engine: &MediaEngine,
    options: &VideoAnalyzeOptions,
    audio_extract_res: Result<(), String>,
    audio_path: &Path,
) -> Option<serde_json::Value> {
    if options.transcribe {
        Some(video_audio_block(engine, audio_extract_res, audio_path).await)
    } else {
        None
    }
}

fn render_video_analyze_result(
    path: &str,
    options: &VideoAnalyzeOptions,
    frames_count: usize,
    timeline: Vec<serde_json::Value>,
    audio: Option<serde_json::Value>,
) -> Result<String, String> {
    let mut out = serde_json::json!({
        "path": path,
        "fps": options.fps,
        "max_frames": options.max_frames,
        "frames_extracted": frames_count,
        "timeline": timeline,
    });
    if let Some(audio) = audio {
        out["audio"] = audio;
    }
    if let Some(prompt) = options.prompt.as_deref() {
        out["prompt"] = serde_json::json!(prompt);
    }
    serde_json::to_string_pretty(&out).map_err(|e| format!("Serialize error: {e}"))
}

async fn video_audio_block(
    engine: &MediaEngine,
    audio_extract_res: Result<(), String>,
    audio_path: &Path,
) -> serde_json::Value {
    let bytes = match audio_extract_res {
        Ok(()) => match tokio::fs::read(audio_path).await {
            Ok(bytes) => bytes,
            Err(e) => return serde_json::json!({ "error": format!("read extracted audio: {e}") }),
        },
        Err(e) => return serde_json::json!({ "error": format!("audio extraction: {e}") }),
    };

    use base64::Engine;
    let attachment = captain_types::media::MediaAttachment {
        media_type: captain_types::media::MediaType::Audio,
        mime_type: "audio/mpeg".to_string(),
        source: captain_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime_type: "audio/mpeg".to_string(),
        },
        size_bytes: bytes.len() as u64,
        context_hint: None,
        batch_size_hint: None,
    };
    match engine.transcribe_audio(&attachment).await {
        Ok(u) => serde_json::json!({
            "transcript": u.description,
            "provider": u.provider,
            "model": u.model,
        }),
        Err(e) => serde_json::json!({ "error": e }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::media::{MediaType, MediaUnderstanding};

    #[test]
    fn parse_video_analyze_options_clamps_frames_and_defaults() {
        let input = serde_json::json!({
            "fps": 2.5,
            "max_frames": 120,
            "prompt": "find scene changes",
            "transcribe": true
        });

        let options = parse_video_analyze_options(&input);

        assert_eq!(options.fps, 2.5);
        assert_eq!(options.max_frames, 60);
        assert_eq!(options.prompt.as_deref(), Some("find scene changes"));
        assert!(options.transcribe);

        let defaults = parse_video_analyze_options(&serde_json::json!({}));
        assert_eq!(defaults.fps, 1.0);
        assert_eq!(defaults.max_frames, 10);
        assert_eq!(defaults.prompt, None);
        assert!(!defaults.transcribe);
    }

    #[test]
    fn frame_context_hint_trims_prompt_and_handles_zero_fps() {
        let options = VideoAnalyzeOptions {
            fps: 0.0,
            max_frames: 3,
            prompt: Some("  summarize action  ".to_string()),
            transcribe: false,
        };

        assert_eq!(
            frame_context_hint(&options, 1, 3).as_deref(),
            Some("summarize action\nFrame 2/3 d'une vidéo, t = 0.0s.")
        );

        let no_prompt = VideoAnalyzeOptions {
            prompt: Some("   ".to_string()),
            ..options
        };
        assert_eq!(
            frame_context_hint(&no_prompt, 2, 3).as_deref(),
            Some("Frame 3/3 d'une vidéo, t = 0.0s.")
        );
    }

    #[test]
    fn video_timeline_from_results_preserves_success_error_and_missing_slots() {
        let timeline = video_timeline_from_results(
            vec![
                Some(Ok(MediaUnderstanding {
                    media_type: MediaType::Image,
                    description: "opening shot".to_string(),
                    provider: "test-provider".to_string(),
                    model: "test-model".to_string(),
                })),
                Some(Err("vision failed".to_string())),
                None,
            ],
            2.0,
        );

        assert_eq!(timeline[0]["index"], 0);
        assert_eq!(timeline[0]["t_seconds"], 0.0);
        assert_eq!(timeline[0]["description"], "opening shot");
        assert_eq!(timeline[0]["provider"], "test-provider");
        assert_eq!(timeline[1]["error"], "vision failed");
        assert_eq!(timeline[2]["t_seconds"], 1.0);
        assert_eq!(timeline[2]["error"], "frame task panicked");
    }
}
