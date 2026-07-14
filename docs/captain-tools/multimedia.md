# Multimedia family

> **Status:** audited (D.16).
> See [`README.md`](README.md) pour l'index et la politique de drift.
> Liste des tools pinned dans [`captain_runtime::captain_docs::MULTIMEDIA_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

La famille Multimedia couvre les deux sens du flux :

- **Verbes input** — analyser, décrire, transcrire (image, audio, vidéo → texte ou métadonnées)
- **Verbes output** — générer, synthétiser (texte → image ou audio)

## Live Calls

Le vrai appel live est distinct des messages audio asynchrones. Dans le web
terminal, `Call` ouvre un flux WebRTC direct : micro navigateur → OpenAI
Realtime → audio de réponse, avec la clé OpenAI gardée côté serveur par
`/api/realtime/calls`. Pendant l'appel, le footer affiche un spectre compact
calculé depuis le flux micro réel via Web Audio `AnalyserNode`; l'animation
n'est pas simulée. Côté coût, Captain active le VAD serveur Realtime et le web
terminal coupe automatiquement l'appel si le micro reste silencieux ou si la
discussion n'a plus d'activité pendant les délais configurés.

Le modèle Realtime n'est pas un second assistant : c'est l'interface audio de
Captain. La session est configurée avec `tool_choice = "required"` pour router
chaque demande substantielle vers l'agent Captain via `captain_message`; après
le retour outil, Realtime ne fait que lire la réponse de Captain. Si
l'utilisateur demande ce qui vient de se passer, Realtime peut appeler
`captain_activity_summary` pour obtenir un résumé court, mais ce flux n'est pas
poussé en continu pour limiter les tokens.

Quand la voix déclenche une action, le web terminal miroite le tour dans une
zone dédiée `Voice` (`voice`, `call`, `captain`) sans injecter de texte dans le
PTY. L'utilisateur peut donc suivre la session pilotée à la voix et reprendre la
main au clavier sans double-exécuter la demande.

La configuration source de vérité est `[voice_call]` :

| Field | Notes |
|---|---|
| `enabled` | Active ou désactive le rail d'appel live. |
| `provider` | `openai` pour l'implémentation WebRTC actuelle. |
| `model` | Modèle Realtime, par défaut `gpt-realtime-2`. |
| `voice` | Voix Realtime, par défaut `marin`. |
| `api_key_env` | Override optionnel ; sinon Captain utilise la résolution OpenAI habituelle (`OPENAI_API_KEY` ou mapping provider). |
| `enable_agent_tool` | Si `true`, le modèle vocal peut appeler `captain_message`; le web terminal route alors la demande à l'agent Captain et renvoie le résultat dans l'appel. |
| `auto_end_silence_secs` | Coupure automatique après silence micro continu. `0` désactive ce garde-fou. Par défaut `90`. |
| `auto_end_inactive_secs` | Coupure automatique après absence d'activité micro/Realtime. `0` désactive ce garde-fou. Par défaut `180`. |
| `instructions` | Prompt système de la voix live. |

Telegram reste un excellent canal de contrôle et d'approbation, mais les appels
vocaux Telegram 1:1 ne sont pas exposés proprement par la Bot API. Pour un vrai
appel live produit, Captain utilise WebRTC/SIP plutôt que les voice notes
Telegram.

## Tools

### `media_pipeline`

Grouped media rail. Runs up to twelve media operations in one call:
`describe_image`, `transcribe_audio`, `video`, `tts`, and `image_generate`.
Use it for Telegram/mobile turns that combine attachments, transcription,
visual analysis, generated voice, or image generation, especially when a final
document/report should be created from the results.

Optional `document` input reuses `document_create` to write a compact media
pipeline report after the media steps complete.

### `image_analyze`

Analyse un fichier image local et retourne ses métadonnées techniques (format, dimensions, taille en octets) ainsi qu'un aperçu base64. Ne pas utiliser pour une description sémantique pure — préférer `media_describe` dans ce cas.

| Field | Required | Notes |
|---|---|---|
| `path` | oui | Chemin absolu ou relatif au workspace (jpg, png, gif, webp, bmp). Path traversal interdit. |
| `prompt` | non | Prompt optionnel pour guider une analyse visuelle par le modèle de vision. Si omis, seules les métadonnées sont retournées. |

Retourne `{ format, width, height, file_size_bytes, analysis? }`.

### `image_generate`

Génère des images à partir d'un prompt textuel. Le mode `provider=auto`
utilise FAL.ai si `FAL_KEY` est disponible (rail rapide), sinon
l'API OpenAI Images via `OPENAI_API_KEY`. Les images générées sont sauvegardées
dans `output/` du workspace et exposées aussi via une URL d'aperçu web locale.

| Field | Required | Notes |
|---|---|---|
| `prompt` | oui | Description textuelle de l'image à générer (max 4 000 caractères). |
| `provider` | non | `auto` (défaut), `fal`, ou `openai`. |
| `model` | non | OpenAI : `gpt-image-2`, `gpt-image-1.5`, `gpt-image-1`, `gpt-image-1-mini`, `dall-e-3`, `dall-e-2`. FAL : `fal-ai/flux-2/klein/9b`, `fal-ai/flux-2-pro`, `fal-ai/gpt-image-1.5`, `fal-ai/nano-banana-pro`, `fal-ai/ideogram/v3`, `fal-ai/recraft/v4/pro/text-to-image`, `fal-ai/qwen-image`. |
| `aspect_ratio` | non | `landscape`, `square`, ou `portrait`. Utilisé quand `size` n'est pas fourni. |
| `size` | non | Override OpenAI. Tailles courantes : `auto`, `1024x1024`, `1536x1024`, `1024x1536`; DALL-E 3 : `1024x1024`, `1024x1792`, `1792x1024`. |
| `quality` | non | OpenAI GPT Image : `auto`, `low`, `medium`, `high`; DALL-E 3 : `standard` ou `hd`; FAL GPT Image : défaut `medium`. |
| `count` | non | Nombre d'images (1–4, défaut 1). DALL-E 3 n'en produit qu'une. Certains modèles FAL rapides ignorent le multi-image. |

Retourne `{ provider, model, images_generated, saved_to, source_urls,
image_urls, revised_prompt? }`.

### `media_describe`

Décrit le contenu d'une image en utilisant un LLM doté de capacités vision. Sélectionne automatiquement le meilleur provider disponible (Anthropic → OpenAI → Gemini). Utiliser pour l'analyse sémantique : OCR, description de scènes, extraction d'informations visuelles. Pour les métadonnées techniques, préférer `image_analyze`.

Les photos reçues depuis Telegram sont normalisées nativement avant le tour
agent : Captain les sauvegarde en fichier local, tente une description
automatique via ce rail, puis transmet au modèle le chemin local, la légende et
la description. Si cette description échoue, le chemin local reste disponible
pour un appel explicite à `media_describe`.

| Field | Required | Notes |
|---|---|---|
| `path` | oui | Chemin du fichier image (relatif au workspace). |
| `prompt` | non | Prompt de guidage (ex : `"Extrais tout le texte visible"`). |

Retourne une description textuelle du contenu visuel.

### `media_transcribe`

Transcrit un fichier audio en texte via reconnaissance vocale. Par défaut,
Captain utilise le provider local `local-whisper` (whisper.cpp small) installé
par `captain voice install`, sans clé API. Les providers API Groq Whisper,
OpenAI Whisper ou ElevenLabs Scribe restent disponibles quand ils sont
explicitement configurés. Formats supportés : mp3, wav, ogg/oga, flac, m4a,
webm.

| Field | Required | Notes |
|---|---|---|
| `path` | oui | Chemin du fichier audio (relatif au workspace). |
| `language` | non | Code ISO-639-1 optionnel (ex : `"fr"`, `"en"`, `"ja"`). |

Retourne la transcription textuelle complète.

### `text_to_speech`

Convertit du texte en audio vocal via le provider TTS configuré dans
`config.toml`. Par défaut, `[tts].provider="local-native"` utilise Kokoro quand
il est prêt et Piper en fallback, sans clé API. Quand `[tts].provider` est
défini, la config est la source de vérité pour le provider et la voix : une
mémoire ou un argument de tool ne doit pas la remplacer. L'audio est sauvegardé
dans `output/` du workspace. Ne pas dépasser 4 096 caractères par appel —
découper si nécessaire.

| Field | Required | Notes |
|---|---|---|
| `text` | oui | Texte à convertir (max 4 096 caractères). |
| `voice` | non | Voix OpenAI uniquement quand `[tts].provider` n'est pas fixé. Si `provider="openai"`, `[tts.openai].voice` gagne. |
| `voice_id` | non | Voice ID ElevenLabs uniquement quand `[tts].provider` n'est pas fixé. Si `provider="elevenlabs"`, `[tts.elevenlabs].voice_id` gagne. |
| `format` | non | `wav` pour le provider local ; `mp3`, `opus`, `aac`, `flac` pour OpenAI. |

Retourne le chemin du fichier audio généré, le provider, la voix réellement
utilisée et `voice_source` (`kokoro`, `piper`, `config` ou `request`). Pour envoyer le résultat
sur Telegram, passer le `saved_to` à `channel_send({channel:"telegram",
file_path})` : Captain route les MP3/WAV via `sendAudio` et les OGG/Opus via
`sendVoice`.

### `speech_to_text`

Transcrit un fichier audio en texte via reconnaissance vocale. Par défaut,
Captain utilise `local-whisper` (whisper.cpp small) en local, puis les providers
API configurés si l'utilisateur les a choisis. Distinct de `media_transcribe` —
utilise le scoping workspace et la résolution de path propres au workspace
courant. Formats supportés : mp3, wav, ogg/oga, flac, m4a, webm.

| Field | Required | Notes |
|---|---|---|
| `path` | oui | Chemin du fichier audio (relatif au workspace). |
| `language` | non | Code ISO-639-1 optionnel (ex : `"fr"`, `"en"`). |

Retourne la transcription textuelle.

### `video_analyze`

Analyse une vidéo locale frame par frame : extrait jusqu'à `max_frames` images PNG à la cadence `fps`, décrit chaque frame via un modèle de vision, et retourne une timeline ordonnée. Optionnellement transcrit la piste audio. Utiliser pour comprendre une courte vidéo ou un screencast. Ne pas utiliser sur de longues vidéos sans réduire `fps` et `max_frames` — chaque frame coûte un appel LLM-vision.

| Field | Required | Notes |
|---|---|---|
| `path` | oui | Chemin local vers le fichier vidéo (mp4, mov, mkv, webm, avi). Path traversal interdit. |
| `fps` | non | Cadence d'extraction (frames/seconde source). Défaut `1.0`. Pour une vidéo longue, choisir `< 1.0` (ex. `0.2` = 1 image / 5 s). |
| `max_frames` | non | Borne dure du nombre total de frames à analyser. Défaut `10`. Cap interne à `60`. |
| `prompt` | non | Indication passée à chaque frame describe (ex : `"Décris l'action principale"`). |
| `transcribe` | non | Si `true`, extrait aussi l'audio MP3 et le transcrit via le provider audio configuré. Défaut `false`. |

Retourne `{ path, fps, max_frames, frames_extracted, timeline:[{index, t_seconds, description, provider, model}], audio? }`.

## Sandbox

**ffmpeg (extraction vidéo / audio)**
Le binaire ffmpeg est téléchargé automatiquement au premier usage de `video_analyze` — aucune installation manuelle requise. Le téléchargement est cross-platform (macOS/Linux/Windows) et vérifié par hash. L'exécutable est placé dans le répertoire de cache du daemon.

**Résolution des clés provider**
Les outils qui appellent un LLM ou une API externe lisent les clés dans l'ordre suivant depuis `~/.captain/secrets.env` :
- Vision / description → `ANTHROPIC_API_KEY`, puis `OPENAI_API_KEY`, puis `GEMINI_API_KEY`
- STT / transcription → `GROQ_API_KEY`, puis `OPENAI_API_KEY`, puis `ELEVENLABS_API_KEY` (`scribe_v2` par défaut, override possible via `ELEVENLABS_STT_MODEL`)
- TTS → `OPENAI_API_KEY` (OpenAI TTS) ou `ELEVENLABS_API_KEY` (ElevenLabs). Avec ElevenLabs, la voix active vient de `[tts.elevenlabs].voice_id` dès que `[tts].provider = "elevenlabs"`.
- Génération d'image → `FAL_KEY` (FAL.ai, rapide/multi-modèles) ou `OPENAI_API_KEY` (OpenAI Images)

Si aucune clé valide n'est disponible pour un provider, l'outil retourne une erreur explicite — pas de fallback silencieux.

**Path traversal**
Tous les outils qui lisent un fichier local (`image_analyze`, `media_describe`, `media_transcribe`, `speech_to_text`, `video_analyze`) passent le chemin par `validate_path`. Les tentatives de sortie du workspace (`../../../etc/passwd`) sont rejetées avec une erreur avant tout accès disque.

**Scoping workspace**
`speech_to_text` et `video_analyze` résolvent les chemins relatifs par rapport au workspace courant de l'agent (le même que `file_read` / `file_write`). `image_analyze`, `media_describe` et `media_transcribe` utilisent le même scoping.

## Limites

- **Cap frames** : `max_frames` est plafonné à `60` en dur dans le code, même si l'appelant demande plus. Chaque frame entraîne un appel LLM-vision distinct — garder `max_frames` bas (≤ 10) pour les vidéos courtes.
- **Taille fichier audio** : les providers STT Groq/OpenAI imposent une limite de ~25 Mo par fichier ; ElevenLabs accepte des fichiers beaucoup plus grands côté API, mais Captain garde 25 Mo pour les vocaux entrants afin d'éviter les téléchargements canal non bornés. Pour les enregistrements longs, découper avec `shell_exec` + `ffmpeg -ss / -t` avant d'appeler `media_transcribe` ou `speech_to_text`.
- **Cascade vision** : si Anthropic est indisponible, l'outil pivote vers OpenAI, puis Gemini. En cas d'échec des trois, l'erreur remonte. Le provider effectivement utilisé est indiqué dans le champ `provider` de chaque frame de la timeline.
- **Pas de streaming** : tous les outils retournent leur résultat en une seule réponse JSON. Pour suivre la progression d'une analyse vidéo longue, utiliser `process_start` + `ffmpeg` manuellement.
- **Encodage audio** : `video_analyze` encode la piste audio en MP3 avant de la transcrire. La qualité d'encodage est fixe (128 kbps) ; pour de l'audio critique, extraire via `shell_exec` + ffmpeg avec vos paramètres, puis appeler `media_transcribe` directement.
- **Génération d'image** : DALL-E 3 ne supporte qu'une seule image par appel (`count` ignoré si > 1). GPT Image et DALL-E 2 supportent jusqu'à 4 côté OpenAI. Côté FAL, `fal-ai/flux-2/klein/9b` reste optimisé pour une image rapide ; choisir un modèle Pro/GPT FAL si le multi-image est important.

## Exemples

### Décrire une image (analyse sémantique)

```
media_describe({
  "path": "screenshots/status-page.png",
  "prompt": "Quels éléments UI sont visibles ? Y a-t-il des erreurs affichées ?"
})
→ "La page de statut affiche 3 agents actifs, un graphe de tokens et une alerte rouge 'Budget dépassé'."
```

### Transcrire un fichier audio

```
media_transcribe({
  "path": "recordings/note-vocale.mp3",
  "language": "fr"
})
→ "Rappel : réunion lundi à 14h, préparer la démo flotte multi-agents."
```

### Analyser une courte vidéo à 1 fps

```
video_analyze({
  "path": "demos/screencast-5s.mp4",
  "fps": 1.0,
  "max_frames": 5,
  "prompt": "Décris l'action principale visible",
  "transcribe": false
})
→ {
  "path": "demos/screencast-5s.mp4",
  "fps": 1.0,
  "max_frames": 5,
  "frames_extracted": 5,
  "timeline": [
    {"index": 0, "t_seconds": 0.0, "description": "Terminal ouvert, cargo build en cours", "provider": "anthropic", "model": "claude-opus-4-5"},
    {"index": 1, "t_seconds": 1.0, "description": "Compilation 87% — aucune erreur visible", ...},
    ...
  ]
}
```
