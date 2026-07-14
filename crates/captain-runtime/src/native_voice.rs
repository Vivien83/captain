//! Native, no-API-key voice runtime discovery.
//!
//! Captain's install/update flow provisions these assets under
//! `~/.captain/native` and `~/.captain/models`. The runtime also accepts
//! explicit env overrides so advanced users and tests can point at custom
//! binaries without changing config.toml.

use serde::Serialize;
use std::path::{Path, PathBuf};

pub const WHISPER_PROVIDER: &str = "local-whisper";
pub const NATIVE_TTS_PROVIDER: &str = "local-native";
pub const WHISPER_MODEL_NAME: &str = "whisper-small";
pub const WHISPER_MODEL_FILE: &str = "ggml-small.bin";
pub const PIPER_VOICE_ID: &str = "fr_FR-siwis-medium";
pub const PIPER_VOICE_FILE: &str = "fr_FR-siwis-medium.onnx";
pub const PIPER_VOICE_CONFIG_FILE: &str = "fr_FR-siwis-medium.onnx.json";
pub const KOKORO_MODEL_FILE: &str = "kokoro-v1.0.onnx";
pub const KOKORO_VOICES_FILE: &str = "voices-v1.0.bin";

pub const WHISPER_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
pub const PIPER_VOICE_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/fr/fr_FR/siwis/medium/fr_FR-siwis-medium.onnx";
pub const PIPER_VOICE_CONFIG_URL: &str = "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/fr/fr_FR/siwis/medium/fr_FR-siwis-medium.onnx.json";
pub const KOKORO_MODEL_URL: &str =
    "https://github.com/nazdridoy/kokoro-tts/releases/download/v1.0.0/kokoro-v1.0.onnx";
pub const KOKORO_VOICES_URL: &str =
    "https://github.com/nazdridoy/kokoro-tts/releases/download/v1.0.0/voices-v1.0.bin";

#[derive(Debug, Clone, Serialize)]
pub struct NativeVoiceStatus {
    pub home_dir: String,
    pub stt_ready: bool,
    pub stt_provider: &'static str,
    pub stt_model: &'static str,
    pub whisper_binary: Option<String>,
    pub whisper_model: Option<String>,
    pub tts_ready: bool,
    pub tts_provider: &'static str,
    pub tts_engine: Option<&'static str>,
    pub kokoro_ready: bool,
    pub kokoro_script: Option<String>,
    pub kokoro_model: Option<String>,
    pub piper_ready: bool,
    pub piper_binary: Option<String>,
    pub piper_voice: Option<String>,
    pub ffmpeg_ready: bool,
    pub install_hint: &'static str,
}

#[derive(Debug, Clone)]
pub struct NativeVoicePaths {
    pub home: PathBuf,
    pub voice_venv_bin: PathBuf,
    pub kokoro_venv_bin: PathBuf,
    pub stt_dir: PathBuf,
    pub tts_dir: PathBuf,
    pub whisper_model: PathBuf,
    pub piper_voice: PathBuf,
    pub piper_voice_config: PathBuf,
    pub kokoro_model: PathBuf,
    pub kokoro_voices: PathBuf,
    pub kokoro_script: PathBuf,
}

pub fn captain_home_dir() -> PathBuf {
    if let Ok(value) = std::env::var("CAPTAIN_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return expand_tilde(trimmed);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".captain")
}

pub fn default_paths() -> NativeVoicePaths {
    let home = captain_home_dir();
    let voice_venv_bin = if cfg!(windows) {
        home.join("native").join("voice-venv").join("Scripts")
    } else {
        home.join("native").join("voice-venv").join("bin")
    };
    let kokoro_venv_bin = if cfg!(windows) {
        home.join("native").join("kokoro-venv").join("Scripts")
    } else {
        home.join("native").join("kokoro-venv").join("bin")
    };
    let stt_dir = home.join("models").join("stt");
    let tts_dir = home.join("models").join("tts");
    NativeVoicePaths {
        home: home.clone(),
        voice_venv_bin,
        kokoro_venv_bin,
        whisper_model: stt_dir.join(WHISPER_MODEL_FILE),
        piper_voice: tts_dir.join(PIPER_VOICE_FILE),
        piper_voice_config: tts_dir.join(PIPER_VOICE_CONFIG_FILE),
        kokoro_model: tts_dir.join(KOKORO_MODEL_FILE),
        kokoro_voices: tts_dir.join(KOKORO_VOICES_FILE),
        kokoro_script: home.join("native").join("kokoro").join("kokoro_tts.py"),
        stt_dir,
        tts_dir,
    }
}

pub fn status() -> NativeVoiceStatus {
    let paths = default_paths();
    let whisper_binary = find_whisper_binary();
    let whisper_model = find_whisper_model();
    let piper_binary = find_piper_binary();
    let piper_voice = find_piper_voice();
    let kokoro_script = find_kokoro_script();
    let kokoro_model = find_kokoro_model();
    let kokoro_voices = find_kokoro_voices();
    let ffmpeg_ready = find_on_path("ffmpeg").is_some();
    let stt_ready = whisper_binary.is_some() && whisper_model.is_some();
    let kokoro_ready = kokoro_script.is_some() && kokoro_model.is_some() && kokoro_voices.is_some();
    let piper_ready = piper_binary.is_some() && piper_voice.is_some();
    let tts_engine = if kokoro_ready {
        Some("kokoro")
    } else if piper_ready {
        Some("piper")
    } else {
        None
    };

    NativeVoiceStatus {
        home_dir: paths.home.display().to_string(),
        stt_ready,
        stt_provider: WHISPER_PROVIDER,
        stt_model: WHISPER_MODEL_NAME,
        whisper_binary: whisper_binary.map(display_path),
        whisper_model: whisper_model.map(display_path),
        tts_ready: tts_engine.is_some(),
        tts_provider: NATIVE_TTS_PROVIDER,
        tts_engine,
        kokoro_ready,
        kokoro_script: kokoro_script.map(display_path),
        kokoro_model: kokoro_model.map(display_path),
        piper_ready,
        piper_binary: piper_binary.map(display_path),
        piper_voice: piper_voice.map(display_path),
        ffmpeg_ready,
        install_hint: "captain voice install",
    }
}

pub fn find_whisper_binary() -> Option<PathBuf> {
    env_path("CAPTAIN_LOCAL_WHISPER_BIN")
        .or_else(|| existing(default_paths().voice_venv_bin.join("whisper-cpp")))
        .or_else(|| existing(default_paths().voice_venv_bin.join("whisper-cpp.exe")))
        .or_else(|| {
            existing(
                default_paths()
                    .home
                    .join("native/whisper.cpp/bin/whisper-cli"),
            )
        })
        .or_else(|| {
            existing(
                default_paths()
                    .home
                    .join("native/whisper.cpp/bin/whisper-cpp"),
            )
        })
        .or_else(|| find_on_path("whisper-cpp"))
        .or_else(|| find_on_path("whisper-cli"))
        .or_else(|| find_on_path("whisper"))
}

pub fn find_whisper_model() -> Option<PathBuf> {
    env_path("CAPTAIN_LOCAL_WHISPER_MODEL")
        .or_else(|| existing(default_paths().whisper_model))
        .or_else(|| existing(default_paths().stt_dir.join("ggml-small-q5_0.bin")))
        .or_else(|| existing(default_paths().stt_dir.join("ggml-base.bin")))
}

pub fn find_piper_binary() -> Option<PathBuf> {
    env_path("CAPTAIN_PIPER_BIN")
        .or_else(|| existing(default_paths().voice_venv_bin.join("piper")))
        .or_else(|| existing(default_paths().voice_venv_bin.join("piper.exe")))
        .or_else(|| existing(default_paths().home.join("native/piper/bin/piper")))
        .or_else(|| find_on_path("piper"))
}

pub fn find_piper_voice() -> Option<PathBuf> {
    env_path("CAPTAIN_PIPER_VOICE")
        .or_else(|| existing(default_paths().piper_voice))
        .or_else(|| existing(default_paths().tts_dir.join("fr_FR-siwis-low.onnx")))
}

pub fn find_kokoro_python() -> Option<PathBuf> {
    env_path("CAPTAIN_KOKORO_PYTHON")
        .or_else(|| existing(default_paths().kokoro_venv_bin.join("python")))
        .or_else(|| existing(default_paths().kokoro_venv_bin.join("python.exe")))
        .or_else(|| find_on_path("python3"))
        .or_else(|| find_on_path("python"))
}

pub fn find_kokoro_script() -> Option<PathBuf> {
    env_path("CAPTAIN_KOKORO_SCRIPT").or_else(|| existing(default_paths().kokoro_script))
}

pub fn find_kokoro_model() -> Option<PathBuf> {
    env_path("CAPTAIN_KOKORO_MODEL").or_else(|| existing(default_paths().kokoro_model))
}

pub fn find_kokoro_voices() -> Option<PathBuf> {
    env_path("CAPTAIN_KOKORO_VOICES").or_else(|| existing(default_paths().kokoro_voices))
}

pub fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_usable_file(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{name}.exe"));
            if is_usable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    existing(expand_tilde(trimmed))
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    if is_usable_file(&path) {
        Some(path)
    } else {
        None
    }
}

fn is_usable_file(path: &Path) -> bool {
    path.is_file()
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn display_path(path: PathBuf) -> String {
    path.display().to_string()
}
