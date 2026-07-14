use std::path::{Path, PathBuf};

use crate::ui;

pub(crate) fn install_native_voice_assets(force: bool) -> Result<(), String> {
    let paths = captain_runtime::native_voice::default_paths();
    std::fs::create_dir_all(&paths.stt_dir)
        .map_err(|e| format!("create {}: {e}", paths.stt_dir.display()))?;
    std::fs::create_dir_all(&paths.tts_dir)
        .map_err(|e| format!("create {}: {e}", paths.tts_dir.display()))?;

    let python = find_python()
        .ok_or("python3/python not found; native voice install needs Python venv support")?;
    let venv = paths.home.join("native").join("voice-venv");
    let venv_python = ensure_python_venv(&python, &venv, force, "native voice venv")?;
    run_command(
        &venv_python,
        [
            "-m",
            "pip",
            "install",
            "--upgrade",
            "pip",
            "whisper.cpp-cli",
            "piper-tts",
        ],
    )?;

    download_if_missing(
        captain_runtime::native_voice::WHISPER_MODEL_URL,
        &paths.whisper_model,
        force,
    )?;
    download_if_missing(
        captain_runtime::native_voice::PIPER_VOICE_URL,
        &paths.piper_voice,
        force,
    )?;
    download_if_missing(
        captain_runtime::native_voice::PIPER_VOICE_CONFIG_URL,
        &paths.piper_voice_config,
        force,
    )?;

    if let Err(e) = install_kokoro_best_effort(&python, &paths, force) {
        ui::check_warn(&format!(
            "Kokoro premium TTS skipped, Piper fallback remains active: {e}"
        ));
    }

    Ok(())
}

fn install_kokoro_best_effort(
    python: &Path,
    paths: &captain_runtime::native_voice::NativeVoicePaths,
    force: bool,
) -> Result<(), String> {
    let python = find_python_with_min_version(3, 10).unwrap_or_else(|| python.to_path_buf());
    let version = python_version(&python).unwrap_or((0, 0));
    if version < (3, 10) {
        return Err(format!(
            "Kokoro needs Python >=3.10; found {}.{}. Piper fallback remains active.",
            version.0, version.1
        ));
    }
    let kokoro_venv = paths.home.join("native").join("kokoro-venv");
    let existing_venv_python = if cfg!(windows) {
        kokoro_venv.join("Scripts").join("python.exe")
    } else {
        kokoro_venv.join("bin").join("python")
    };
    if existing_venv_python.exists()
        && python_version(&existing_venv_python)
            .map(|version| version < (3, 10))
            .unwrap_or(true)
    {
        std::fs::remove_dir_all(&kokoro_venv)
            .map_err(|e| format!("remove incompatible kokoro venv: {e}"))?;
    }
    let venv_python = ensure_python_venv(&python, &kokoro_venv, force, "kokoro venv")?;
    run_command(&venv_python, ["-m", "pip", "install", "--upgrade", "pip"])?;
    run_command(
        &venv_python,
        [
            "-m",
            "pip",
            "install",
            "--upgrade",
            "kokoro-onnx",
            "soundfile",
        ],
    )?;
    download_if_missing(
        captain_runtime::native_voice::KOKORO_MODEL_URL,
        &paths.kokoro_model,
        force,
    )?;
    download_if_missing(
        captain_runtime::native_voice::KOKORO_VOICES_URL,
        &paths.kokoro_voices,
        force,
    )?;
    if let Some(parent) = paths.kokoro_script.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create kokoro dir: {e}"))?;
    }
    let script = r#"#!/usr/bin/env python3
import argparse

from kokoro_onnx import Kokoro
import soundfile as sf

parser = argparse.ArgumentParser()
parser.add_argument("--model", required=True)
parser.add_argument("--voices", required=True)
parser.add_argument("--voice", required=True)
parser.add_argument("--output", required=True)
parser.add_argument("--text", required=True)
args = parser.parse_args()

kokoro = Kokoro(args.model, args.voices)
samples, sample_rate = kokoro.create(args.text, voice=args.voice, speed=1.0, lang="fr-fr")
sf.write(args.output, samples, sample_rate)
"#;
    std::fs::write(&paths.kokoro_script, script)
        .map_err(|e| format!("write kokoro script: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&paths.kokoro_script)
            .map_err(|e| format!("metadata kokoro script: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&paths.kokoro_script, perms)
            .map_err(|e| format!("chmod kokoro script: {e}"))?;
    }
    Ok(())
}

pub(crate) fn find_python() -> Option<PathBuf> {
    [
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
        "python",
    ]
    .iter()
    .find_map(|name| find_command_path(name))
}

fn find_python_with_min_version(major: u32, minor: u32) -> Option<PathBuf> {
    [
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
        "python",
    ]
    .iter()
    .filter_map(|name| find_command_path(name))
    .find(|path| {
        python_version(path)
            .map(|version| version >= (major, minor))
            .unwrap_or(false)
    })
}

pub(crate) fn ensure_python_venv(
    python: &Path,
    venv: &Path,
    force: bool,
    label: &str,
) -> Result<PathBuf, String> {
    if force && venv.exists() {
        std::fs::remove_dir_all(venv).map_err(|e| format!("remove {label}: {e}"))?;
    }

    let venv_python = venv_python_path(venv);
    if venv.exists() && !venv_python.exists() {
        std::fs::remove_dir_all(venv).map_err(|e| format!("remove broken {label}: {e}"))?;
    }

    if !venv.exists() {
        create_python_venv(python, venv, label)?;
    }

    if !python_module_importable(&venv_python, "pip") {
        let repaired = run_command(&venv_python, ["-m", "ensurepip", "--upgrade"]).is_ok()
            && python_module_importable(&venv_python, "pip");
        if !repaired {
            std::fs::remove_dir_all(venv).map_err(|e| format!("remove pip-less {label}: {e}"))?;
            create_python_venv(python, venv, label)?;
        }
    }

    if !python_module_importable(&venv_python, "pip") {
        return Err(format!(
            "{label} was created but pip is unavailable. Install python3-venv/python3-pip, then retry `captain voice install`."
        ));
    }

    Ok(venv_python)
}

fn create_python_venv(python: &Path, venv: &Path, label: &str) -> Result<(), String> {
    if let Some(parent) = venv.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {label} parent: {e}"))?;
    }
    let venv_arg = venv.to_string_lossy().to_string();
    run_command(python, ["-m", "venv", venv_arg.as_str()])
        .map_err(|e| format!("create {label}: {e}"))
}

fn venv_python_path(venv: &Path) -> PathBuf {
    if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    }
}

pub(crate) fn python_module_importable(python: &Path, module: &str) -> bool {
    std::process::Command::new(python)
        .args(["-c", &format!("import {module}")])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn python_version(path: &Path) -> Option<(u32, u32)> {
    let output = std::process::Command::new(path)
        .args([
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let mut parts = stdout.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

fn find_command_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn run_command<I, S>(program: &Path, args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to launch {}: {e}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    Err(format!("{} failed: {detail}", program.display()))
}

fn download_if_missing(url: &str, path: &Path, force: bool) -> Result<(), String> {
    if path.exists() && !force {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let curl = find_command_path("curl").ok_or("curl not found; cannot download voice model")?;
    let tmp = path.with_extension("download");
    let output = std::process::Command::new(&curl)
        .args(["-L", "--fail", "--retry", "2", "-o"])
        .arg(&tmp)
        .arg(url)
        .output()
        .map_err(|e| format!("failed to launch curl: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("download failed for {url}: {}", stderr.trim()));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("install {}: {e}", path.display()))?;
    Ok(())
}
