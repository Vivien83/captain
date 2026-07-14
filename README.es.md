<p align="center">
  <img src="assets/logo.png" alt="Captain" width="280">
</p>

<h1 align="center">Captain</h1>

<p align="center"><b>El Agent OS autoalojado con disciplina de producción.</b></p>

<p align="center">
  <img src="https://img.shields.io/badge/Built%20in-Rust-B7410E?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-green?style=for-the-badge" alt="License">
  <img src="https://img.shields.io/badge/Platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows%20%C2%B7%20Docker-blue?style=for-the-badge" alt="Platforms">
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.fr.md">Français</a> ·
  <b>Español</b> ·
  <a href="README.zh.md">中文</a>
</p>

**Un operador de IA persistente en tu propio hardware.** Captain es un daemon
en Rust que conserva conversaciones, proyectos, memoria, tareas programadas y
estado de agentes entre sesiones y reinicios. Puede ejecutar herramientas
reales, delegar en agentes aislados, exponer un agente mediante una API segura
y seguir siendo observable mientras el trabajo continúa en segundo plano. Las
aprobaciones, presupuestos, guardas de bucle, checkpoints y el registro de
auditoría limitan esa autonomía. Ejecútalo en macOS, Linux, Windows, un VPS o
Docker, y úsalo desde el terminal, la aplicación web Control autenticada,
Telegram o Discord.

> **Alfa pública:** Captain está en desarrollo activo. Espera bugs, aristas
> sin pulir y cambios incompatibles entre versiones preliminares. Conserva
> copias de seguridad, revisa cada capacidad concedida y no uses esta alfa
> para cargas críticas.

<table>
<tr><td width="220"><b>Un binario, un daemon</b></td><td>Un núcleo Rust compilado orquesta agentes, herramientas, memoria, canales, programaciones y aprobaciones. Arranca en segundos, consume poco en reposo, sobrevive a reinicios como servicio nativo (launchd/systemd), y se actualiza a sí mismo — pídeselo por chat, aprueba, listo.</td></tr>
<tr><td><b>Trabajo duradero</b></td><td>Proyectos, objetivos, checkpoints, workflows y ejecuciones de herramientas desacopladas se persisten. Tras un reinicio, el trabajo incompleto queda visible como <code>interrupted</code> en vez de desaparecer o repetirse a ciegas.</td></tr>
<tr><td><b>Ejecución real, vigilada</b></td><td>Shell, archivos, SSH, navegador, investigación web, código, documentos y medios. Las llamadas sensibles usan aprobaciones; los patrones críticos se bloquean; los presupuestos limitan tokens, coste y frecuencia. Las lecturas independientes pueden ejecutarse en paralelo, mientras las dependencias y los efectos secundarios siguen ordenados.</td></tr>
<tr><td><b>Memoria que sigue la conversación</b></td><td>Recuerdo de sesiones, hechos duraderos del usuario, estado de proyectos, MemPalace, grafo de conocimiento y embeddings ONNX locales opcionales aportan contexto acotado sin reinyectar todo el historial en cada turno.</td></tr>
<tr><td><b>Cualquier modelo, sin ataduras</b></td><td>Codex con tu suscripción de ChatGPT, Anthropic, OpenAI, Mistral, Groq, Gemini, OpenRouter y modelos locales vía Ollama. Captain descubre el catálogo y las credenciales configuradas sin depender de cifras fijas. Para Codex, una actualización cada hora muestra los modelos nuevos en Control y, si está configurado, Telegram; Captain nunca cambia de modelo sin tu decisión explícita y tu estrategia de sesión.</td></tr>
<tr><td><b>Seis centros operativos</b></td><td>Chat, Projects, Automation, Learning, Capabilities y Status forman la superficie principal compartida por el TUI y Control. Automation agrupa Workflows, Triggers, Crons, Aprobaciones y Webhooks.</td></tr>
<tr><td><b>Agentes como servicios</b></td><td>Cada agente puede recibir ingress externo autenticado y emitir callbacks HTTP firmados. Captain prepara el ingress automáticamente e indica la URL externa que aún hace falta para activar el egress.</td></tr>
<tr><td><b>Operable como software real</b></td><td><code>captain doctor</code> explica qué está roto y cómo arreglarlo. Snapshots y reinicio de fábrica (con respaldo primero, siempre). Registro de auditoría encadenado por hash. Endpoints de salud. Un asistente de configuración que termina con un daemon funcionando y verificado — no un muro de próximos pasos.</td></tr>
</table>

---

## Instalación rápida

Versión pública preliminar actual:
[v0.1.0-alpha.1](https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.1).
Imagen Docker inmutable: `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.1`;
canal alfa móvil: `ghcr.io/vivien83/captain-agent-os:alpha`.

### macOS / Linux / VPS

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.1/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.1 bash
```

El repositorio oficial, los assets, los checksums y la imagen son públicos. No
se necesita token de GitHub ni inicio de sesión en el registro.

El instalador descarga un paquete precompilado y verificado por checksum
para tu plataforma (sin compilación, sin toolchain), verifica el CLI de
principio a fin, y ejecuta una configuración guiada que **termina con
Captain realmente en ejecución** como servicio en segundo plano.

Los assets de release cubren `aarch64` y `x86_64` para macOS y Linux, además
de un zip CLI `x86_64-pc-windows-msvc`. Cada archivo incluye su SHA-256 y un
manifiesto de plataforma; la release también publica un manifiesto agregado y
los instaladores Unix.

> **Firma de la alfa:** los archivos y checksums están publicados, pero los
> binarios de macOS solo llevan una firma ad hoc y no están notarizados por
> Apple. El CLI de Windows no está firmado con Authenticode. Verifica el
> archivo SHA-256 y espera una aprobación explícita del sistema en el primer
> inicio.

### VPS sin interfaz (totalmente no interactivo)

```bash
export ANTHROPIC_API_KEY=...       # o cualquier clave de proveedor soportado
export TELEGRAM_BOT_TOKEN=...      # opcional — ver más abajo
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.1/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.1 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 bash
```

El perfil `vps` instala un servicio systemd, lo inicia, y valida su salud.
Si hay un token de Telegram presente, Captain lo valida contra la API de
Telegram, descubre tu chat a partir de los mensajes pendientes del bot, y
**te envía un mensaje de confirmación — tu primer contacto con tu agente
ocurre en tu teléfono, segundos después de la instalación.**

### VPS sin interfaz con tu suscripción de ChatGPT (Codex, sin clave API)

Codex es el proveedor por defecto integrado de Captain — no necesitas
`ANTHROPIC_API_KEY` ni nada similar, solo tu sesión de ChatGPT
Plus/Pro/Pro+. `CAPTAIN_START=0` instala todo (binario, servicio systemd)
sin arrancar aún el daemon, para que la comprobación de disponibilidad de
abajo no se ejecute antes de que hayas iniciado sesión:

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.1/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.1 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 CAPTAIN_START=0 bash

captain login codex        # muestra una URL + un código — ábrela en tu teléfono, sin necesidad de navegador local
systemctl start captain    # instalación no-root: systemctl --user start captain
```

### Docker

La alfa pública proporciona imágenes `linux/amd64` y `linux/arm64` en GitHub
Container Registry. No hace falta autenticación para descargarlas:

```bash
docker run -d --name captain --restart unless-stopped \
  -p 50051:50051 \
  -v captain-data:/root/.captain \
  -e CAPTAIN_LISTEN=0.0.0.0:50051 \
  -e MISTRAL_API_KEY \
  ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.1
```

El primer arranque genera la clave API del daemon y la persiste — junto
con todo el estado — en un volumen con nombre que sobrevive a las
actualizaciones de imagen. El runtime local de embeddings se provisiona en la
imagen.

Para compilar desde el código o usar un perfil de acceso al host, clona el
repositorio y usa los archivos Compose. El servicio base también lleva el
nombre de la imagen GHCR, por lo que `docker compose pull` puede consumir una
release publicada:

| Perfil | Acceso al host |
|---|---|
| *por defecto* | ninguno — solo volumen de estado |
| `personal` | Escritorio/Documentos/Descargas + SSH al host |
| `trusted` | `$HOME` completo + socket de Docker |
| `yolo` | privilegiado, red del host, sistema de archivos completo |

```bash
git clone https://github.com/Vivien83/captain.git && cd captain
MISTRAL_API_KEY=... docker compose up -d --build

# Ejemplo con acceso controlado a directorios personales
docker compose -f docker-compose.yml -f docker-compose.personal.yml up -d --build
```

---

## Primeros pasos

```bash
captain setup       # asistente guiado: proveedor → preferencias → canales → Captain en marcha
captain             # interfaz de terminal completa
captain chat        # chat rápido de terminal
captain doctor      # diagnostica cualquier cosa, con soluciones
captain update      # auto-actualización (o simplemente pídele a Captain que se actualice)
captain status      # daemon, agentes, canales, presupuestos, disco, salud
```

Proveedores recomendados para empezar:

- **Codex** — `captain auth login codex`. Usa tu suscripción de ChatGPT;
  sin clave API que gestionar.
- **Claude** — exporta `ANTHROPIC_API_KEY` antes de la configuración.

La primera conversación activa una breve entrevista de bienvenida (nombre,
idioma, estilo, límites) — una sola vez, en todas las interfaces,
almacenada de forma duradera.

La aplicación web Control autenticada está disponible por defecto en
`http://127.0.0.1:50051/`. Sus seis centros reflejan el TUI, de modo que
proyectos, automatizaciones, capacidades y salud permanecen en el mismo lugar.
El terminal experto sigue disponible en `http://127.0.0.1:50051/terminal`.

---

## CLI vs mensajería

Ejecuta el daemon una vez; háblale desde cualquier lugar. Los canales son
**denegados por defecto**: cada adaptador requiere una lista blanca
explícita de usuarios antes de responder a nadie.

| Acción | Terminal | Telegram / Discord |
|---|---|---|
| Hablar con Captain | `captain chat` o el TUI | mensaje al bot |
| Aprobar una acción sensible | panel de aprobaciones del TUI | botones inline |
| Interrumpir el trabajo en curso | `Esc` / `Ctrl+C` | `/stop` |
| Estado / reinicio del daemon | `captain status` / `captain service restart` | `status` / `restart` en el chat |
| Voz | `captain voice` (Whisper STT local + Kokoro TTS) | enviar una nota de voz |
| Actualizar Captain | `captain update` | "actualízate" → aprobación → hecho |

---

## Qué puedes pedirle

```text
Revisa mi VPS: disco, memoria, servicios caídos — arregla lo que sea seguro arreglar.
Investiga X en toda la web y produce un informe PDF con fuentes.
Vigila esta carpeta y resúmeme los documentos nuevos por Telegram.
Cada mañana a las 8: mi calendario, el clima, cualquier cosa rara en los logs.
Conéctate por SSH al servidor de backup y verifica que el job de anoche realmente corrió.
Actualízate.
```

Bajo el capó, las herramientas integradas se seleccionan semánticamente para
que solo los esquemas útiles lleguen al modelo. Captain también admite skills
gobernadas, servidores MCP, delegación multiagente, workflows, automatización
de navegador y ejecuciones duraderas que el agente puede revisar, cancelar u
ordenar mediante dependencias.

---

## Documentación

| Guía | Qué cubre |
|---|---|
| [Getting Started](docs/getting-started.md) | Instalación → configuración → primera conversación |
| [Configuration](docs/configuration.md) | `config.toml`, proveedores, modelos, todas las opciones |
| [CLI Reference](docs/cli-reference.md) | Todos los comandos y flags |
| [Providers](docs/providers.md) | Proveedores de modelos, auth, respaldo, enrutamiento |
| [Channel Adapters](docs/channel-adapters.md) | Configuración de Telegram, Discord, Signal, Email |
| [Security Profiles](docs/SECURITY-PROFILES.md) | Políticas de aprobación, modos de ejecución, aislamiento |
| [Built-in Tools](docs/captain-tools/) | Documentación de herramientas por familia |
| [Architecture](docs/architecture.md) | Crates, bucle del agente, diseño del kernel |
| [API Reference](docs/api-reference.md) | Endpoints REST, auth, streaming |
| [VPS Deployment](docs/deployment/github-vps-install.md) | Instalaciones headless, proxy inverso, HTTPS |
| [MCP & A2A](docs/mcp-a2a.md) | Servidores de herramientas externos, agente-a-agente |
| [Troubleshooting](docs/troubleshooting.md) | Problemas comunes y sus soluciones |
| [Notas de la versión 0.1.0-alpha.1](docs/releases/v0.1.0-alpha.1.md) | Alcance preliminar y limitaciones conocidas |
| [Docs Status (DOC2)](docs/DOCS_STATUS.md) | Contratos actuales, superficies congeladas y documentos históricos |

> Las guías detalladas en `docs/` están actualmente solo en inglés.

---

## Postura de seguridad

- La API se vincula a `127.0.0.1` por defecto y **se niega a iniciar** en
  una interfaz pública sin autenticación configurada.
- El acceso web/API requiere una sesión iniciada o una clave API bearer;
  el editor de configuración web está autenticado.
- Las herramientas sensibles pasan por el flujo de aprobación; los
  patrones de shell hiper-críticos se bloquean o fuerzan una aprobación
  puntual sin importar la política.
- Presupuestos por agente: tokens, coste por hora/día/mes, frecuencia de
  llamadas a herramientas.
- Guardián de bucles: disyuntores por repetición, ping-pong, y fallos
  consecutivos.
- Listas blancas de canales denegadas por defecto; registro de auditoría
  encadenado por hash; los secretos viven en `secrets.env` o en la bóveda
  cifrada, nunca en archivos de configuración.

El estado vive bajo `~/.captain/` — `config.toml` es la única fuente de
verdad, recargada en caliente ante cualquier cambio.

---

## Desarrollo

```bash
cargo test --workspace              # suite completa
cargo build --release -p captain-cli
scripts/release-readiness.sh         # gate local completa de release
CAPTAIN_VERSION=vX.Y.Z scripts/release-all.sh  # los 5 objetivos CLI en local
CAPTAIN_VERSION=vX.Y.Z scripts/publish-release-local.sh
docker build --build-arg CAPTAIN_BUILD_VERSION=vX.Y.Z -t captain:vX.Y.Z .
```

`release-all.sh` compila los dos bundles de macOS, los dos de Linux y el bundle
CLI de Windows; el cross-build de Windows usa `cargo-xwin`, LLVM y NASM. Después de una gate
de release limpia, `publish-release-local.sh` valida los 20 assets, sube la
rama actual, compila y publica la imagen GHCR `linux/amd64` + `linux/arm64`, y
después publica el tag y la GitHub Release. La imagen reutiliza los dos
binarios Linux verificados en vez de recompilar Captain bajo emulación. Antes
de ensamblar la imagen, el publicador prepara desde la caché local de Captain un
snapshot FastEmbed fijado por checksum en `dist/docker/`, ignorado por Git. No
se confirma en el repositorio ni se añade a los 20 assets, y Docker lo verifica
de nuevo sin depender de un CDN de modelos disponible.
Autentica `gh` una vez con
`gh auth refresh -h github.com -s read:packages,write:packages`; no pases un
token en la línea de comandos. El workflow release de GitHub es un fallback
manual explícito y los pushes de tags no lo inician. La CI sigue disponible
mediante ejecución manual explícita para formato, Clippy estricto, auditorías
de seguridad/secretos y checks/tests del workspace.

---

## Licencia

Doble licencia bajo [MIT](LICENSE-MIT) o [Apache 2.0](LICENSE-APACHE), a
tu elección.
