//! Static multimedia tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn multimedia_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = media_pipeline_tool_definitions();
    definitions.extend(image_tool_definitions());
    definitions.extend(audio_tool_definitions());
    definitions.extend(video_tool_definitions());
    definitions
}

fn media_pipeline_tool_definitions() -> Vec<ToolDefinition> {
    vec![media_pipeline_tool_definition()]
}

fn image_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        image_analyze_tool_definition(),
        image_generate_tool_definition(),
        media_describe_tool_definition(),
    ]
}

fn audio_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        media_transcribe_tool_definition(),
        text_to_speech_tool_definition(),
        speech_to_text_tool_definition(),
    ]
}

fn video_tool_definitions() -> Vec<ToolDefinition> {
    vec![video_analyze_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn media_pipeline_tool_definition() -> ToolDefinition {
    tool_definition(
        "media_pipeline",
        "[MEDIA PIPELINE] Traite plusieurs médias en un appel: describe_image, transcribe_audio, video, tts, image_generate. Peut aussi créer un document de synthèse avec le champ document. À utiliser pour audio/image/video multiples sans multiplier les appels tool.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "maxItems": 12,
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "describe_image, transcribe_audio, video, tts, image_generate" },
                            "type": { "type": "string", "description": "Alias de action." },
                            "path": { "type": "string" },
                            "prompt": { "type": "string" },
                            "language": { "type": "string" },
                            "text": { "type": "string" },
                            "voice": { "type": "string" },
                            "voice_id": { "type": "string" },
                            "format": { "type": "string" }
                        },
                        "required": ["action"]
                    }
                },
                "document": { "type": "object", "description": "Optionnel: input document_create partiel pour générer un rapport des résultats." },
                "stop_on_error": { "type": "boolean", "default": false },
                "preview_chars": { "type": "integer", "description": "Taille de preview par item, défaut 5000." }
            },
            "required": ["items"]
        }),
    )
}

fn image_analyze_tool_definition() -> ToolDefinition {
    tool_definition(
        "image_analyze",
        "Analyse un fichier image local et retourne ses métadonnées techniques (format, dimensions, taille en octets) ainsi qu'un aperçu base64. Utiliser quand il faut inspecter les propriétés d'une image ou la faire analyser par un modèle de vision. Ne pas utiliser pour une description sémantique pure — préférer `media_describe` dans ce cas. Retourne un objet JSON avec format, width, height, file_size_bytes et, si un prompt est fourni, une analyse textuelle du contenu visuel.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin absolu ou relatif au workspace vers le fichier image (formats supportés : jpg, png, gif, webp, bmp)" },
                "prompt": { "type": "string", "description": "Prompt optionnel pour guider l'analyse visuelle par le modèle de vision (ex: 'Décris ce que tu vois', 'Extrais tout le texte visible'). Si omis, seules les métadonnées sont retournées." }
            },
            "required": ["path"]
        }),
    )
}

fn image_generate_tool_definition() -> ToolDefinition {
    tool_definition(
        "image_generate",
        "Génère des images à partir d'un prompt textuel. Provider auto: FAL.ai si FAL_KEY est disponible (rail rapide), sinon OpenAI Images API via OPENAI_API_KEY. Utiliser pour créer des illustrations, logos, maquettes visuelles ou images de contenu. Les images générées sont sauvegardées dans output/ du workspace. Ne pas utiliser pour de l'édition d'images existantes. Retourne le chemin du fichier image généré et les URLs d'aperçu.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Text description of the image to generate (max 4000 chars)" },
                "provider": { "type": "string", "description": "Provider: 'auto' (default), 'fal', or 'openai'. Auto uses FAL_KEY first when no model is forced, otherwise OPENAI_API_KEY." },
                "model": { "type": "string", "description": "OpenAI: 'gpt-image-2', 'gpt-image-1.5', 'gpt-image-1', 'gpt-image-1-mini', 'dall-e-3', 'dall-e-2'. FAL: 'fal-ai/flux-2/klein/9b', 'fal-ai/flux-2-pro', 'fal-ai/gpt-image-1.5', 'fal-ai/nano-banana-pro', 'fal-ai/ideogram/v3', 'fal-ai/recraft/v4/pro/text-to-image', 'fal-ai/qwen-image'." },
                "aspect_ratio": { "type": "string", "description": "Aspect ratio when size is omitted: 'landscape', 'square', or 'portrait'." },
                "size": { "type": "string", "description": "OpenAI size override. Common GPT Image sizes: 'auto', '1024x1024', '1536x1024', '1024x1536'. DALL-E 3: '1024x1024', '1792x1024', '1024x1792'." },
                "quality": { "type": "string", "description": "Quality: OpenAI GPT Image 'auto', 'low', 'medium', 'high'; DALL-E 3 'standard' or 'hd'; FAL GPT Image defaults to 'medium'." },
                "count": { "type": "integer", "description": "Number of images to generate (1-4, default: 1). DALL-E 3 only supports 1." }
            },
            "required": ["prompt"]
        }),
    )
}

fn media_describe_tool_definition() -> ToolDefinition {
    tool_definition(
        "media_describe",
        "Décrit le contenu d'une image en utilisant un LLM doté de capacités vision. Sélectionne automatiquement le meilleur provider disponible (Anthropic, OpenAI ou Gemini). Utiliser pour l'analyse sémantique d'images (OCR, description de scènes, extraction d'informations visuelles). Pour les métadonnées techniques (dimensions, format), préférer image_analyze. Retourne une description textuelle du contenu visuel.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the image file (relative to workspace)" },
                "prompt": { "type": "string", "description": "Optional prompt to guide the description (e.g., 'Extract all text from this image')" }
            },
            "required": ["path"]
        }),
    )
}

fn media_transcribe_tool_definition() -> ToolDefinition {
    tool_definition(
        "media_transcribe",
        "Transcrit un fichier audio en texte via reconnaissance vocale. Sélectionne automatiquement le meilleur provider disponible (Groq Whisper, OpenAI Whisper ou ElevenLabs Scribe). Utiliser pour transcrire des messages vocaux, des interviews ou des notes audio. Formats supportés : mp3, wav, ogg/oga, flac, m4a, webm. Retourne la transcription textuelle complète.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the audio file (relative to workspace). Supported: mp3, wav, ogg/oga, flac, m4a, webm." },
                "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
            },
            "required": ["path"]
        }),
    )
}

fn text_to_speech_tool_definition() -> ToolDefinition {
    tool_definition(
        "text_to_speech",
        "Convertit du texte en audio vocal via le provider TTS configuré dans config.toml (source de vérité). Par défaut Captain utilise la voix native locale sans clé API (Kokoro si prêt, Piper en fallback). Utiliser directement pour générer messages vocaux, podcasts courts ou notifications audio, sans chercher un outil audio d'abord. Ne pas utiliser pour des textes de plus de 4096 caractères — découper en plusieurs appels si nécessaire. L'audio est sauvegardé dans output/ du workspace. Si [tts].provider est défini, la voix configurée gagne toujours sur les mémoires et sur les arguments voice/voice_id. Retourne le chemin du fichier généré et la voix réellement utilisée.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "Texte à convertir en audio (max 4096 caractères). Pour des textes plus longs, découper en plusieurs appels." },
                "voice": { "type": "string", "enum": ["alloy", "echo", "fable", "onyx", "nova", "shimmer"], "description": "Voix OpenAI optionnelle seulement quand [tts].provider n'est pas fixé. Si un provider est configuré, la voix de config.toml gagne." },
                "voice_id": { "type": "string", "description": "Voice ID ElevenLabs optionnel seulement quand [tts].provider n'est pas fixé. Si provider='elevenlabs', [tts.elevenlabs].voice_id est la source de vérité." },
                "format": { "type": "string", "enum": ["wav", "mp3", "opus", "aac", "flac"], "description": "Format audio de sortie. Le provider local natif retourne WAV; OpenAI peut retourner mp3/opus/aac/flac." }
            },
            "required": ["text"]
        }),
    )
}

fn speech_to_text_tool_definition() -> ToolDefinition {
    tool_definition(
        "speech_to_text",
        "Transcrit un fichier audio en texte via reconnaissance vocale. Par défaut Captain utilise la voix native locale (whisper.cpp small) sans clé API; fallback possible vers Groq Whisper, OpenAI Whisper ou ElevenLabs Scribe si configurés. Utiliser directement pour transcrire messages vocaux, notes audio ou enregistrements, sans chercher un outil audio d'abord. Formats supportés : mp3, wav, ogg/oga, flac, m4a, webm. Pour la synthèse vocale inverse, utiliser text_to_speech. Retourne la transcription textuelle.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the audio file (relative to workspace)" },
                "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
            },
            "required": ["path"]
        }),
    )
}

fn video_analyze_tool_definition() -> ToolDefinition {
    tool_definition(
        "video_analyze",
        "Analyse une vidéo locale frame par frame : extrait jusqu'à `max_frames` images (PNG) à la cadence `fps`, décrit chaque frame via un modèle de vision, et retourne une timeline ordonnée. Optionnellement transcrit la piste audio. Utiliser pour comprendre une courte vidéo ou un screencast. Ne pas utiliser sur de longues vidéos sans réduire `fps` et `max_frames` — chaque frame coûte un appel LLM-vision. Le binaire ffmpeg est téléchargé automatiquement au premier usage. Retourne un JSON `{ path, fps, max_frames, frames_extracted, timeline:[{index,t_seconds,description,provider,model}], audio? }`.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin local vers le fichier vidéo (mp4, mov, mkv, webm, avi). Path traversal interdit." },
                "fps": { "type": "number", "description": "Cadence d'extraction (frames par seconde de la vidéo source). Default 1.0 (1 image/sec). Choisir < 1.0 pour une vidéo longue (ex 0.2 = 1 image / 5 s)." },
                "max_frames": { "type": "integer", "description": "Borne dure du nombre total de frames à analyser. Default 10. Cap interne à 60 même si l'appelant demande plus (anti-coût-explosion)." },
                "prompt": { "type": "string", "description": "Indication optionnelle passée à chaque frame describe (ex: 'Décris l'action principale', 'Y a-t-il du texte visible ?'). Si omis, description neutre." },
                "transcribe": { "type": "boolean", "description": "Si true, extrait aussi l'audio en MP3 et le transcrit via le provider audio configuré (Groq/OpenAI/parakeet-mlx). Default false." }
            },
            "required": ["path"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multimedia_tool_definitions_keep_public_order() {
        let definitions = multimedia_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "media_pipeline",
                "image_analyze",
                "image_generate",
                "media_describe",
                "media_transcribe",
                "text_to_speech",
                "speech_to_text",
                "video_analyze",
            ]
        );
    }

    #[test]
    fn multimedia_tool_definitions_keep_pipeline_and_image_contracts() {
        let definitions = multimedia_tool_definitions();
        let pipeline = tool(&definitions, "media_pipeline");
        let image_generate = tool(&definitions, "image_generate");
        let media_describe = tool(&definitions, "media_describe");

        assert_eq!(required_fields(pipeline), vec!["items"]);
        assert_eq!(
            integer_field(property(pipeline, "items"), "maxItems"),
            Some(12)
        );
        assert_eq!(
            required_fields_from(
                property(pipeline, "items")
                    .get("items")
                    .expect("items should define item schema")
            ),
            vec!["action"]
        );
        assert_eq!(
            boolean_field(property(pipeline, "stop_on_error"), "default"),
            Some(false)
        );
        assert_eq!(required_fields(image_generate), vec!["prompt"]);
        assert_contains(&image_generate.description, "FAL_KEY");
        assert_contains(&image_generate.description, "OPENAI_API_KEY");
        assert_eq!(required_fields(media_describe), vec!["path"]);
    }

    #[test]
    fn multimedia_tool_definitions_keep_audio_and_video_contracts() {
        let definitions = multimedia_tool_definitions();
        let media_transcribe = tool(&definitions, "media_transcribe");
        let tts = tool(&definitions, "text_to_speech");
        let stt = tool(&definitions, "speech_to_text");
        let video = tool(&definitions, "video_analyze");

        assert_eq!(required_fields(media_transcribe), vec!["path"]);
        assert_eq!(required_fields(tts), vec!["text"]);
        assert_eq!(
            enum_values(property(tts, "voice")),
            vec!["alloy", "echo", "fable", "onyx", "nova", "shimmer"]
        );
        assert_eq!(
            enum_values(property(tts, "format")),
            vec!["wav", "mp3", "opus", "aac", "flac"]
        );
        assert_eq!(required_fields(stt), vec!["path"]);
        assert_eq!(required_fields(video), vec!["path"]);
        assert_contains(
            property(video, "max_frames")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "Cap interne à 60",
        );
    }

    fn tool<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        definitions
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        required_fields_from(&tool.input_schema)
    }

    fn required_fields_from(schema: &Value) -> Vec<String> {
        schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn property<'a>(tool: &'a ToolDefinition, name: &str) -> &'a Value {
        tool.input_schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("{} should define property {name}", tool.name))
    }

    fn enum_values(property: &Value) -> Vec<String> {
        property
            .get("enum")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn integer_field(value: &Value, name: &str) -> Option<u64> {
        value.get(name).and_then(Value::as_u64)
    }

    fn boolean_field(value: &Value, name: &str) -> Option<bool> {
        value.get(name).and_then(Value::as_bool)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
