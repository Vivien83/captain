//! V.1 — Helper d'extraction de frames vidéo via `ffmpeg-sidecar` (#184).
//!
//! Première brique de la pipeline d'analyse vidéo de Captain. L'API publique :
//! [`extract_frames`] valide les bornes, lance ffmpeg en sous-processus et
//! produit `frame_0001.jpg`, `frame_0002.jpg`… sur disque (V.8c #184 — JPEG
//! q≈85, indissociable visuellement du PNG pour le vision LLM mais ~5×
//! plus petit). Le tool `video_analyze` (V.3) consommera ensuite ces frames
//! via `MediaEngine`.
//!
//! ## Auto-download du binaire
//!
//! En distribution publique on ne peut pas supposer que `ffmpeg` est dans le
//! PATH. [`ensure_ffmpeg`] délègue à `ffmpeg_sidecar::download::auto_download`
//! qui cherche d'abord dans le PATH, puis dans le dossier sidecar (à côté de
//! l'exécutable Rust), et télécharge sinon le binaire pour la plateforme
//! courante (macOS Intel/M1, Linux x86_64/ARM, Windows). Lazy : exécuté au
//! 1er usage de l'analyse vidéo, pas au boot du daemon.
//!
//! ## Garde-fous (#184 V.1)
//!
//! - `max_frames` est dur — empêche un appelant qui demande 10 fps sur une
//!   vidéo de 1h de faire exploser coût LLM et stockage.
//! - Bornes invalides (`max_frames=0`, `fps<=0`, `fps NaN/Inf`, vidéo
//!   absente) → `Err` synchrone, ffmpeg n'est pas lancé.
//! - Le sous-processus est drainé jusqu'à terminaison (sinon ses buffers
//!   stderr peuvent le bloquer).
//!
//! ## V.2 — Audio
//!
//! [`extract_audio`] complète la pipeline : utile pour transcription
//! (Whisper / Groq / parakeet) en plus de l'analyse frame par frame.
//! Output MP3 via `ffmpeg -vn -acodec mp3` — tradeoff : taille raisonnable,
//! lisible par tous les transcripteurs, qualité suffisante pour la voix.

use ffmpeg_sidecar::{
    command::FfmpegCommand,
    download::auto_download,
    event::{FfmpegEvent, LogLevel},
};
use std::path::{Path, PathBuf};

/// Maximum long-edge (in pixels) for a frame uploaded to a vision LLM.
///
/// Anthropic, OpenAI and Gemini all bill vision tokens by image dimension.
/// Anthropic's own recommendation is "1568 px max for the long edge" — we
/// pick 1024 because (a) it's measurably indistinguishable from 1568 for
/// scene-description / action-narration workloads, and (b) it cuts upload
/// bandwidth and per-frame token cost roughly 5× vs. a 1080p screenshot.
pub const VISION_MAX_LONG_EDGE: u32 = 1024;

/// Configuration pour [`extract_frames`].
#[derive(Debug, Clone)]
pub struct VideoFrameExtractConfig {
    /// Cadence d'extraction (frames par seconde de la vidéo source).
    /// Default : `1.0`.
    pub fps: f32,
    /// Borne maximale de frames produites. Default : `30`. Garde-fou
    /// contre l'explosion de coût LLM-vision et de stockage disque.
    pub max_frames: usize,
    /// Répertoire de sortie pour `frame_%04d.jpg` (V.8c #184). Créé
    /// récursivement s'il n'existe pas.
    pub output_dir: PathBuf,
}

impl Default for VideoFrameExtractConfig {
    fn default() -> Self {
        Self {
            fps: 1.0,
            max_frames: 30,
            output_dir: std::env::temp_dir().join("captain_video_frames"),
        }
    }
}

/// Garantit que le binaire `ffmpeg` est utilisable. No-op si déjà dans le
/// PATH ou cache. Sinon télécharge ~30 Mo (premier appel uniquement).
/// Wrappe l'appel synchrone de la crate dans `spawn_blocking` pour ne pas
/// bloquer le runtime async.
pub async fn ensure_ffmpeg() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        auto_download().map_err(|e| format!("ffmpeg auto-download failed: {e}"))
    })
    .await
    .map_err(|e| format!("ensure_ffmpeg join error: {e}"))?
}

/// Extrait jusqu'à `cfg.max_frames` frames depuis `video_path` à la cadence
/// `cfg.fps` vers `cfg.output_dir`. Retourne la liste triée des PNG produits.
///
/// L'appelant doit avoir appelé [`ensure_ffmpeg`] au préalable (en pratique :
/// au boot paresseux du tool `video_analyze`).
pub async fn extract_frames(
    video_path: &Path,
    cfg: &VideoFrameExtractConfig,
) -> Result<Vec<PathBuf>, String> {
    if !video_path.exists() {
        return Err(format!("video file not found: {}", video_path.display()));
    }
    if cfg.max_frames == 0 {
        return Err("max_frames must be > 0".to_string());
    }
    if !cfg.fps.is_finite() || cfg.fps <= 0.0 {
        return Err(format!("fps must be finite and > 0 (got {})", cfg.fps));
    }

    tokio::fs::create_dir_all(&cfg.output_dir)
        .await
        .map_err(|e| format!("create_dir_all({}) failed: {e}", cfg.output_dir.display()))?;

    let video = video_path.to_path_buf();
    let output_dir = cfg.output_dir.clone();
    let fps = cfg.fps;
    let max = cfg.max_frames;

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let output_pattern = output_dir
            .join("frame_%04d.jpg")
            .to_string_lossy()
            .to_string();

        // V.8c (#184): JPEG q≈85 instead of PNG. ffmpeg's mjpeg encoder maps
        // `-q:v` 1..31 to (very loosely) quality 100..0 — q:v=3 sits around
        // JPEG quality 85, which is visually indistinguishable from lossless
        // for the vision LLM but ~5× smaller on disk and over the wire.
        let mut child = FfmpegCommand::new()
            .input(video.to_string_lossy().as_ref())
            .args(["-vf", &format!("fps={fps}")])
            .args(["-frames:v", &max.to_string()])
            .args(["-q:v", "3"])
            .args(["-y"])
            .output(&output_pattern)
            .spawn()
            .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

        // Keep every error/fatal line instead of only the last one: ffmpeg
        // often prints a specific root cause (e.g. a bad codec/path) followed
        // by a generic summary line, and overwriting hides the useful part.
        let mut error_lines: Vec<String> = Vec::new();
        let iter = child
            .iter()
            .map_err(|e| format!("ffmpeg iter init failed: {e}"))?;
        for event in iter {
            match event {
                FfmpegEvent::Error(msg)
                | FfmpegEvent::Log(LogLevel::Error | LogLevel::Fatal, msg) => {
                    error_lines.push(msg);
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .map_err(|e| format!("ffmpeg wait failed: {e}"))?;
        if !status.success() {
            let detail = if error_lines.is_empty() {
                format!("ffmpeg exited with status {status:?}")
            } else {
                error_lines.join(" | ")
            };
            return Err(format!("ffmpeg failed: {detail}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("extract_frames join error: {e}"))??;

    let mut entries = tokio::fs::read_dir(&cfg.output_dir)
        .await
        .map_err(|e| format!("read_dir failed: {e}"))?;
    let mut paths = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("read_dir entry failed: {e}"))?
    {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("jpg") {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Extrait la piste audio de `video_path` vers `output_path` en MP3.
///
/// Ré-encode via `ffmpeg -i <video> -vn -acodec mp3 -y <output>`. Le
/// répertoire parent de `output_path` est créé s'il n'existe pas.
///
/// L'appelant doit avoir appelé [`ensure_ffmpeg`] au préalable.
///
/// # Erreurs
/// - Vidéo source absente sur disque
/// - ffmpeg sort en non-zéro (ex. : aucune piste audio dans la vidéo)
pub async fn extract_audio(video_path: &Path, output_path: &Path) -> Result<(), String> {
    if !video_path.exists() {
        return Err(format!("audio source not found: {}", video_path.display()));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create_dir_all({}) failed: {e}", parent.display()))?;
        }
    }

    let video = video_path.to_path_buf();
    let out = output_path.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut child = FfmpegCommand::new()
            .input(video.to_string_lossy().as_ref())
            .args(["-vn"])
            .args(["-acodec", "mp3"])
            .args(["-y"])
            .output(out.to_string_lossy().as_ref())
            .spawn()
            .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

        // Keep every error/fatal line instead of only the last one: ffmpeg
        // often prints a specific root cause (e.g. a bad codec/path) followed
        // by a generic summary line, and overwriting hides the useful part.
        let mut error_lines: Vec<String> = Vec::new();
        let iter = child
            .iter()
            .map_err(|e| format!("ffmpeg iter init failed: {e}"))?;
        for event in iter {
            match event {
                FfmpegEvent::Error(msg)
                | FfmpegEvent::Log(LogLevel::Error | LogLevel::Fatal, msg) => {
                    error_lines.push(msg);
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .map_err(|e| format!("ffmpeg wait failed: {e}"))?;
        if !status.success() {
            let detail = if error_lines.is_empty() {
                format!("ffmpeg exited with status {status:?}")
            } else {
                error_lines.join(" | ")
            };
            return Err(format!("ffmpeg failed: {detail}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("extract_audio join error: {e}"))??;

    Ok(())
}

/// Convert any supported audio/video input into a mono 16 kHz WAV file.
///
/// This is the stable pre-processing format expected by whisper.cpp. It is
/// also what Telegram voice notes need after download because they arrive as
/// OGG/OPUS while local STT engines usually expect PCM WAV.
pub async fn transcode_audio_to_wav(input_path: &Path, output_path: &Path) -> Result<(), String> {
    if !input_path.exists() {
        return Err(format!("audio source not found: {}", input_path.display()));
    }

    // ffmpeg cannot read and overwrite the same file in one pass (it fails
    // with an opaque "Error opening output files: Invalid argument"), so
    // reject this case explicitly instead of letting ffmpeg produce a
    // confusing error a caller has to reverse-engineer.
    if input_path == output_path {
        return Err(format!(
            "transcode_audio_to_wav: input and output path are identical ({}); \
             the caller must materialize the source under a different filename",
            input_path.display()
        ));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create_dir_all({}) failed: {e}", parent.display()))?;
        }
    }

    let input = input_path.to_path_buf();
    let out = output_path.to_path_buf();

    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut child = FfmpegCommand::new()
            .input(input.to_string_lossy().as_ref())
            .args(["-vn"])
            .args(["-ac", "1"])
            .args(["-ar", "16000"])
            .args(["-y"])
            .output(out.to_string_lossy().as_ref())
            .spawn()
            .map_err(|e| format!("ffmpeg spawn failed: {e}"))?;

        // Keep every error/fatal line instead of only the last one: ffmpeg
        // often prints a specific root cause (e.g. a bad codec/path) followed
        // by a generic summary line, and overwriting hides the useful part.
        let mut error_lines: Vec<String> = Vec::new();
        let iter = child
            .iter()
            .map_err(|e| format!("ffmpeg iter init failed: {e}"))?;
        for event in iter {
            match event {
                FfmpegEvent::Error(msg)
                | FfmpegEvent::Log(LogLevel::Error | LogLevel::Fatal, msg) => {
                    error_lines.push(msg);
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .map_err(|e| format!("ffmpeg wait failed: {e}"))?;
        if !status.success() {
            let detail = if error_lines.is_empty() {
                format!("ffmpeg exited with status {status:?}")
            } else {
                error_lines.join(" | ")
            };
            return Err(format!("ffmpeg failed: {detail}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("transcode_audio_to_wav join error: {e}"))??;

    Ok(())
}

/// Resize the image at `input_path` so its longest edge ≤ `max` pixels.
/// Mutates the file in place. No-op if the image is already small enough.
///
/// CPU-bound; the caller wraps this in `spawn_blocking` (see
/// [`downscale_to_long_edge_async`]) so the async runtime stays responsive.
///
/// V.8a (#184): cuts vision token cost ~2.5× and upload bandwidth ~5× for
/// 1080p+ frames. Lanczos3 is the standard high-quality downscale filter
/// — produces sharper results than bilinear/triangle for the kind of
/// scene/action content Captain analyses.
pub fn downscale_to_long_edge(input_path: &Path, max: u32) -> Result<(), String> {
    let img = image::open(input_path).map_err(|e| format!("read {}: {e}", input_path.display()))?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) <= max {
        // Already small enough — leave the file untouched.
        return Ok(());
    }
    let resized = img.resize(max, max, image::imageops::FilterType::Lanczos3);
    resized
        .save(input_path)
        .map_err(|e| format!("save {}: {e}", input_path.display()))?;
    Ok(())
}

/// Async wrapper around [`downscale_to_long_edge`]. Runs the CPU work on
/// the blocking thread pool so it doesn't stall the tokio runtime.
pub async fn downscale_to_long_edge_async(input_path: PathBuf, max: u32) -> Result<(), String> {
    tokio::task::spawn_blocking(move || downscale_to_long_edge(&input_path, max))
        .await
        .map_err(|e| format!("downscale join error: {e}"))?
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
