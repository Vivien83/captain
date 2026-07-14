//! Image, media, video, and voice dispatch.

use std::path::Path;

use super::{
    tool_image_analyze, tool_image_generate, tool_media_describe, tool_media_pipeline,
    tool_media_transcribe, tool_speech_to_text, tool_text_to_speech, tool_video_analyze,
};

pub(crate) async fn dispatch_media_tool(
    tool_name: &str,
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
    tool_use_id: &str,
) -> Result<String, String> {
    match tool_name {
        "image_analyze" => tool_image_analyze(input).await,
        "media_pipeline" => {
            tool_media_pipeline(input, media_engine, tts_engine, workspace_root, tool_use_id).await
        }
        "media_describe" => tool_media_describe(input, media_engine).await,
        "media_transcribe" => tool_media_transcribe(input, media_engine).await,
        "video_analyze" => {
            tool_video_analyze(input, media_engine, workspace_root, tool_use_id).await
        }
        "image_generate" => tool_image_generate(input, workspace_root).await,
        "text_to_speech" => tool_text_to_speech(input, tts_engine, workspace_root).await,
        "speech_to_text" => tool_speech_to_text(input, media_engine, workspace_root).await,
        other => Err(format!("Unknown media/voice tool: {other}")),
    }
}
