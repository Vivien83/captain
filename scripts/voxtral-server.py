#!/usr/bin/env python3.11
"""
Captain STT WebSocket Server — Multi-model support.

Models: whisper-small, whisper-large-v3, voxtral-4b, mistral-api
Client selects model via {"type":"config","model":"whisper-small"} message.

Usage:
    python3.11 scripts/voxtral-server.py [--port 8766]

Protocol (WebSocket):
    Client sends: binary PCM 16kHz 16-bit mono, or JSON commands
    Server sends: JSON {"type":"transcript","text":"...","final":false}
"""

import argparse
import asyncio
import json
import struct
import sys
import tempfile
import os

try:
    import websockets
except ImportError:
    sys.exit("pip install websockets")

MISTRAL_API_KEY = os.environ.get("MISTRAL_API_KEY", "")
SAMPLE_RATE = 16000

# ─── Model registry ─────────────────────────────────────────────────────────

MODELS = {}  # name → loaded model/processor pair


def _ensure_whisper(model_name: str):
    """Lazy-load a Whisper model via mlx-whisper."""
    if model_name in MODELS:
        return True
    try:
        import mlx_whisper
        hf_id = {
            "whisper-small": "mlx-community/whisper-small-mlx",
            "whisper-large-v3": "mlx-community/whisper-large-v3-mlx",
        }.get(model_name)
        if not hf_id:
            return False
        # mlx_whisper loads on first transcribe call, just validate import
        MODELS[model_name] = {"type": "whisper", "hf_id": hf_id}
        print(f"[STT] Registered {model_name} ({hf_id})")
        return True
    except ImportError:
        print(f"[STT] mlx-whisper not installed, {model_name} unavailable")
        return False


def _ensure_voxtral():
    """Lazy-load Voxtral 4B via voxmlx."""
    if "voxtral-4b" in MODELS:
        return True
    try:
        from voxmlx import load_model as vox_load
        path = "mlx-community/Voxtral-Mini-4B-Realtime-6bit"
        model = vox_load(path)
        MODELS["voxtral-4b"] = {"type": "voxmlx", "model": model, "path": path}
        print(f"[STT] Loaded voxtral-4b ({path})")
        return True
    except ImportError:
        try:
            from mlx_voxtral import VoxtralForConditionalGeneration, VoxtralProcessor
            m = VoxtralForConditionalGeneration.from_pretrained("mistralai/Voxtral-Mini-3B-2507")
            p = VoxtralProcessor.from_pretrained("mistralai/Voxtral-Mini-3B-2507")
            MODELS["voxtral-4b"] = {"type": "mlx_voxtral", "model": m, "processor": p}
            print("[STT] Loaded voxtral-4b (mlx_voxtral 3B)")
            return True
        except ImportError:
            print("[STT] Neither voxmlx nor mlx_voxtral installed, voxtral-4b unavailable")
            return False


def available_models() -> list[str]:
    """Return list of models that can be loaded."""
    models = []
    try:
        import mlx_whisper  # noqa: F401
        models.extend(["whisper-small", "whisper-large-v3"])
    except ImportError:
        pass
    try:
        import voxmlx  # noqa: F401
        models.append("voxtral-4b")
    except ImportError:
        try:
            import mlx_voxtral  # noqa: F401
            models.append("voxtral-4b")
        except ImportError:
            pass
    if MISTRAL_API_KEY:
        models.append("mistral-api")
    return models


# ─── Transcription ───────────────────────────────────────────────────────────

def write_wav(audio_bytes: bytes) -> str:
    f = tempfile.NamedTemporaryFile(suffix=".wav", delete=False)
    num_samples = len(audio_bytes) // 2
    data_size = num_samples * 2
    f.write(b'RIFF')
    f.write(struct.pack('<I', 36 + data_size))
    f.write(b'WAVE')
    f.write(b'fmt ')
    f.write(struct.pack('<IHHIIHH', 16, 1, 1, SAMPLE_RATE, SAMPLE_RATE * 2, 2, 16))
    f.write(b'data')
    f.write(struct.pack('<I', data_size))
    f.write(audio_bytes)
    f.close()
    return f.name


def transcribe(audio_bytes: bytes, model_name: str, language: str = "fr") -> str:
    if len(audio_bytes) < 1000:
        return ""
    wav_path = write_wav(audio_bytes)
    try:
        return _transcribe_with(wav_path, model_name, language)
    finally:
        os.unlink(wav_path)


def _transcribe_with(wav_path: str, model_name: str, language: str) -> str:
    try:
        if model_name.startswith("whisper"):
            import mlx_whisper
            info = MODELS.get(model_name, {})
            hf_id = info.get("hf_id", "mlx-community/whisper-small-mlx")
            result = mlx_whisper.transcribe(wav_path, path_or_hf_repo=hf_id, language=language)
            return result.get("text", "").strip()

        elif model_name == "voxtral-4b":
            info = MODELS.get("voxtral-4b", {})
            if info.get("type") == "voxmlx":
                from voxmlx import transcribe as vox_transcribe
                return vox_transcribe(wav_path, info["path"]).strip()
            elif info.get("type") == "mlx_voxtral":
                m, p = info["model"], info["processor"]
                inputs = p.apply_transcrition_request(language=language, audio=wav_path, task="transcribe")
                outputs = m.generate(**inputs, max_new_tokens=1024, temperature=0.0)
                return p.decode(outputs[0][inputs.input_ids.shape[1]:], skip_special_tokens=True).strip()

        elif model_name == "mistral-api":
            return _transcribe_api(wav_path)

    except Exception as e:
        print(f"[STT:{model_name}] Error: {e}")
    return ""


def _transcribe_api(wav_path: str) -> str:
    import urllib.request
    boundary = "----CaptainBoundary"
    with open(wav_path, "rb") as f:
        file_data = f.read()
    body = (
        f"--{boundary}\r\n"
        f'Content-Disposition: form-data; name="file"; filename="audio.wav"\r\n'
        f"Content-Type: audio/wav\r\n\r\n"
    ).encode() + file_data + (
        f"\r\n--{boundary}\r\n"
        f'Content-Disposition: form-data; name="model"\r\n\r\n'
        f"voxtral-mini-latest\r\n"
        f"--{boundary}--\r\n"
    ).encode()
    req = urllib.request.Request(
        "https://api.mistral.ai/v1/audio/transcriptions",
        data=body,
        headers={
            "Authorization": f"Bearer {MISTRAL_API_KEY}",
            "Content-Type": f"multipart/form-data; boundary={boundary}",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            data = json.loads(resp.read())
            return data.get("text", "").strip()
    except Exception as e:
        print(f"[API] Error: {e}")
        return ""


# ─── WebSocket handler ───────────────────────────────────────────────────────

async def handle_client(websocket):
    pending_buffer = bytearray()
    text_parts = []
    language = "fr"
    model_name = "whisper-small"  # default
    transcribing = False
    chunk_secs = 2
    chunk_bytes = SAMPLE_RATE * 2 * chunk_secs
    print(f"Client connected (default model={model_name})")

    try:
        async for message in websocket:
            if isinstance(message, bytes):
                pending_buffer.extend(message)
                if not transcribing and len(pending_buffer) >= chunk_bytes:
                    transcribing = True
                    chunk_data = bytes(pending_buffer)
                    pending_buffer.clear()
                    text = await asyncio.get_event_loop().run_in_executor(
                        None, transcribe, chunk_data, model_name, language
                    )
                    transcribing = False
                    if text:
                        text_parts.append(text)
                        full_text = " ".join(text_parts)
                        await websocket.send(json.dumps({
                            "type": "transcript", "text": full_text, "final": False
                        }))
            else:
                try:
                    cmd = json.loads(message)
                    if cmd.get("type") == "config":
                        language = cmd.get("language", "fr")
                        req_model = cmd.get("model", model_name)
                        # Validate and load requested model
                        if req_model in available_models():
                            if req_model.startswith("whisper"):
                                _ensure_whisper(req_model)
                            elif req_model == "voxtral-4b":
                                _ensure_voxtral()
                            model_name = req_model
                            chunk_secs = 2 if model_name in ("whisper-small", "mistral-api") else 3
                            chunk_bytes = SAMPLE_RATE * 2 * chunk_secs
                        await websocket.send(json.dumps({
                            "type": "config_ok",
                            "language": language,
                            "model": model_name,
                            "available": available_models(),
                        }))
                    elif cmd.get("type") == "end":
                        if pending_buffer:
                            text = await asyncio.get_event_loop().run_in_executor(
                                None, transcribe, bytes(pending_buffer), model_name, language
                            )
                            pending_buffer.clear()
                            if text:
                                text_parts.append(text)
                        full_text = " ".join(text_parts)
                        await websocket.send(json.dumps({
                            "type": "transcript", "text": full_text, "final": True
                        }))
                    elif cmd.get("type") == "clear":
                        pending_buffer.clear()
                        text_parts.clear()
                    elif cmd.get("type") == "ping":
                        await websocket.send(json.dumps({
                            "type": "pong", "available": available_models()
                        }))
                except json.JSONDecodeError:
                    pass
    except websockets.exceptions.ConnectionClosed:
        pass
    print(f"Client disconnected (model={model_name})")


async def main(port: int):
    avail = available_models()
    print(f"Captain STT server — available models: {avail}")
    if not avail:
        print("WARNING: No STT models available. Install mlx-whisper or set MISTRAL_API_KEY.")
    # Pre-load default model
    if "whisper-small" in avail:
        _ensure_whisper("whisper-small")
    async with websockets.serve(handle_client, "0.0.0.0", port):
        print(f"Listening on ws://0.0.0.0:{port}")
        await asyncio.Future()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8766)
    args = parser.parse_args()
    asyncio.run(main(args.port))
