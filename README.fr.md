<p align="center">
  <img src="assets/logo.png" alt="Captain" width="280">
</p>

<h1 align="center">Captain</h1>

<p align="center"><b>L'Agent OS auto-hébergé, avec une discipline de production.</b></p>

<p align="center">
  <a href="https://captainagent.fr/"><b>captainagent.fr</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Built%20in-Rust-B7410E?style=for-the-badge&logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-green?style=for-the-badge" alt="License">
  <img src="https://img.shields.io/badge/Platforms-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows%20%C2%B7%20Docker-blue?style=for-the-badge" alt="Platforms">
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <b>Français</b> ·
  <a href="README.es.md">Español</a> ·
  <a href="README.zh.md">中文</a>
</p>

**Un opérateur IA persistant, sur votre propre machine.** Captain est un
daemon Rust qui conserve les conversations, projets, souvenirs, tâches
planifiées et états d'agents entre les sessions et les redémarrages. Il peut
exécuter de vrais outils, déléguer à des agents isolés, exposer un agent par
API sécurisée et rester observable pendant les travaux en arrière-plan. Les
approbations, budgets, gardes anti-boucle, checkpoints et la piste d'audit
encadrent cette autonomie. Faites-le tourner sur macOS, Linux, Windows, un
VPS ou Docker, puis utilisez-le depuis le terminal, l'interface Control web
authentifiée, Telegram ou Discord.

> **Alpha publique :** Captain est en développement actif. Attendez-vous à des
> bugs, des aspérités et des ruptures entre les préversions. Gardez des
> sauvegardes, vérifiez chaque capacité accordée et ne confiez aucune charge
> critique à cette alpha.

<table>
<tr><td width="220"><b>Un binaire, un daemon</b></td><td>Un cœur Rust compilé orchestre agents, outils, mémoire, canaux, planifications et approbations. Démarre en quelques secondes, consomme peu au repos, survit aux redémarrages en tant que service natif (launchd/systemd), et se met à jour lui-même — demandez-le lui dans le chat, approuvez, terminé.</td></tr>
<tr><td><b>Travail durable</b></td><td>Projets, goals, checkpoints, workflows et appels d'outils détachés sont persistés. Après un redémarrage, un travail détaché incomplet devient inspectable comme <code>interrupted</code> au lieu de disparaître ou d'être rejoué à l'aveugle.</td></tr>
<tr><td><b>Exécution réelle, encadrée</b></td><td>Shell, fichiers, SSH, navigateur, recherche web, code, documents et médias. Les appels sensibles utilisent les approbations ; les motifs shell critiques sont bloqués ; les budgets limitent tokens, coût et fréquence. Les lectures indépendantes peuvent s'exécuter en parallèle, tandis que les dépendances et effets de bord restent ordonnés.</td></tr>
<tr><td><b>Une mémoire qui suit l'échange</b></td><td>Rappel de sessions, faits utilisateur durables, état des projets, graphe de connaissances et embeddings ONNX locaux optionnels fournissent un contexte borné sans réinjecter tout l'historique à chaque tour. Les faits acceptés entrent d'abord dans un journal local durable, restent disponibles pendant une panne MemPalace et se resynchronisent automatiquement avec un backoff borné.</td></tr>
<tr><td><b>N'importe quel modèle, aucun verrouillage</b></td><td>Codex via votre abonnement ChatGPT, Anthropic, OpenAI, Mistral, Groq, Gemini, OpenRouter et modèles locaux via Ollama. Captain découvre le catalogue et les identifiants réellement configurés sans dépendre de compteurs figés. Pour Codex, une actualisation horaire signale les nouveaux modèles dans Control et, s'il est configuré, Telegram ; Captain ne bascule jamais sans votre décision explicite et votre choix de stratégie de session.</td></tr>
<tr><td><b>Six hubs opérationnels</b></td><td>Chat, Projects, Automation, Learning, Capabilities et Status forment la surface primaire commune au TUI et à Control. Automation regroupe Workflows, Triggers, Crons, Approbations et Webhooks.</td></tr>
<tr><td><b>Agents exposés comme services</b></td><td>Chaque agent peut recevoir un ingress externe authentifié et émettre des callbacks HTTP signés. Captain prépare l'ingress automatiquement et indique précisément l'URL de callback externe encore nécessaire pour rendre l'egress prêt.</td></tr>
<tr><td><b>Opérable comme un vrai logiciel</b></td><td><code>captain doctor</code> explique ce qui est cassé et comment le réparer. Snapshots et reset usine (sauvegarde d'abord, toujours). Piste d'audit chaînée par hash. Endpoints de santé. Un assistant de configuration qui se termine avec un daemon en cours d'exécution et vérifié — pas un mur de prochaines étapes.</td></tr>
</table>

---

## Installation rapide

Préversion publique actuelle :
[v0.1.0-alpha.4](https://github.com/Vivien83/captain/releases/tag/v0.1.0-alpha.4).
Image Docker immuable : `ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4` ;
canal alpha mobile : `ghcr.io/vivien83/captain-agent-os:alpha`.

### macOS / Linux / VPS

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.4/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.4 bash
```

Le dépôt officiel, les assets, les checksums et l'image sont publics. Aucun
token GitHub ni login au registre n'est requis.

L'installeur télécharge un bundle précompilé et vérifié par checksum pour
votre plateforme (pas de compilation, pas de toolchain), vérifie le CLI de
bout en bout, et lance une configuration guidée qui **se termine avec
Captain réellement en cours d'exécution** en tant que service en arrière-plan.

La même installation provisionne le runtime mémoire géré par Captain avant le
démarrage du daemon : uv 0.11.28, CPython 3.13.14 isolé, MemPalace 3.5.0 et un
lock de dépendances gelé et lié par checksum. Aucun Python système, `pip
install` manuel ni clé API secondaire n'est nécessaire. `captain memory
doctor` le vérifie réellement ; au démarrage, Captain répare un runtime absent,
corrompu ou insuffisamment protégé, puis vérifie une vraie lecture sémantique.
Si la réparation échoue, Captain ne se déclare pas prêt pour la production
sans mémoire sémantique.

Les releases couvrent `aarch64` et `x86_64` pour macOS et Linux, ainsi qu'un
zip CLI `x86_64-pc-windows-msvc`. Chaque archive possède son fichier SHA-256
et son manifeste de plateforme ; la release contient aussi le manifeste
agrégé et les installateurs Unix.

> **Signature de l'alpha :** les archives et checksums sont publiés, mais les
> binaires macOS sont seulement signés ad hoc et ne sont pas notarisés par
> Apple. Le CLI Windows n'est pas signé avec Authenticode. Vérifiez le fichier
> SHA-256 et attendez-vous à une approbation explicite du système au premier
> lancement.

### VPS headless (entièrement non-interactif)

```bash
export ANTHROPIC_API_KEY=...       # ou toute clé de provider supportée
export TELEGRAM_BOT_TOKEN=...      # optionnel — voir ci-dessous
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.4/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.4 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 bash
```

Le profil `vps` installe un service systemd, le démarre, et valide sa
santé. Si un token Telegram est présent, Captain le valide auprès de l'API
Telegram, découvre votre chat depuis les messages en attente du bot, et
**vous envoie un message de confirmation — votre premier contact avec votre
agent se fait sur votre téléphone, quelques secondes après l'installation.**

### VPS headless avec votre abonnement ChatGPT (Codex, sans clé API)

Codex est le provider par défaut natif de Captain — pas besoin de
`ANTHROPIC_API_KEY` ou équivalent, juste votre connexion ChatGPT
Plus/Pro/Pro+. `CAPTAIN_START=0` installe tout (binaire, service systemd)
sans démarrer le daemon tout de suite, pour que la vérification de
disponibilité ci-dessous ne tourne pas avant que vous vous soyez connecté :

```bash
curl -fsSL https://github.com/Vivien83/captain/releases/download/v0.1.0-alpha.4/install.sh \
  | CAPTAIN_VERSION=v0.1.0-alpha.4 CAPTAIN_PROFILE=vps CAPTAIN_YES=1 CAPTAIN_START=0 bash

captain login codex        # affiche une URL + un code — ouvrez-la sur votre téléphone, pas besoin de navigateur local
systemctl start captain    # install non-root : systemctl --user start captain
```

### Docker

L'alpha publique fournit les images `linux/amd64` et `linux/arm64` sur GitHub
Container Registry. Leur téléchargement ne demande aucune authentification :

```bash
docker run -d --name captain --restart unless-stopped \
  -p 50051:50051 \
  -v captain-data:/root/.captain \
  -e CAPTAIN_LISTEN=0.0.0.0:50051 \
  ghcr.io/vivien83/captain-agent-os:v0.1.0-alpha.4
```

Le premier démarrage génère la clé API du daemon et la persiste — avec tout
l'état — dans un volume nommé qui survit aux mises à jour de l'image. Le
runtime d'embeddings locaux et le runtime MemPalace géré sont provisionnés dans
l'image. L'entrypoint exécute le doctor sémantique à chaque démarrage et répare
un runtime absent, corrompu ou insuffisamment protégé avant de lancer le daemon,
y compris lorsqu'un bind mount masque l'état préchargé de l'image.

Le fichier Compose public monte volontairement uniquement le volume d'état
Captain. Il n'expose ni le système de fichiers hôte, ni le socket Docker, ni
l'espace PID, ni le mode privilégié. Pour lancer l'image immuable :

```bash
git clone https://github.com/Vivien83/captain.git && cd captain
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.4 docker compose pull
CAPTAIN_IMAGE_TAG=v0.1.0-alpha.4 docker compose up -d
```

Configurez le provider choisi après le premier démarrage. Tout accès à l'hôte
doit être un changement de déploiement explicite et relu localement ; les
anciens overlays d'accès large ne font pas partie de la release publique.

---

## Prise en main

```bash
captain setup       # assistant guidé : provider → préférences → canaux → Captain en marche
captain             # interface terminal complète
captain chat        # chat terminal rapide
captain doctor      # diagnostique tout, avec correctifs
captain update      # auto-mise à jour (ou demandez simplement à Captain de se mettre à jour)
captain status      # daemon, agents, canaux, budgets, disque, santé
```

Providers recommandés pour démarrer :

- **Codex** — `captain auth login codex`. Utilise votre abonnement ChatGPT ;
  aucune clé API à gérer.
- **Claude** — exportez `ANTHROPIC_API_KEY` avant la configuration.

La première conversation déclenche un court entretien d'accueil (nom,
langue, style, limites) — une seule fois, sur toutes les interfaces, stocké
durablement.

L'interface Control web authentifiée est disponible par défaut sur
`http://127.0.0.1:50051/`. Ses six hubs reflètent ceux du TUI : les projets,
automations, capacités et diagnostics restent au même endroit. Le terminal
expert reste accessible sur `http://127.0.0.1:50051/terminal`.

---

## CLI vs messagerie

Lancez le daemon une fois ; parlez-lui de n'importe où. Les canaux sont
**refusés par défaut** : chaque adaptateur exige une liste blanche
d'utilisateurs explicite avant de répondre à qui que ce soit.

| Action | Terminal | Telegram / Discord |
|---|---|---|
| Parler à Captain | `captain chat` ou le TUI | message au bot |
| Approuver une action sensible | panneau d'approbations du TUI | boutons inline |
| Interrompre le travail en cours | `Esc` / `Ctrl+C` | `/stop` |
| Statut / redémarrage du daemon | `captain status` / `captain service restart` | `status` / `restart` dans le chat |
| Voix | `captain voice` (Whisper STT local + Kokoro TTS) | envoyer une note vocale |
| Mettre à jour Captain | `captain update` | « mets-toi à jour » → approbation → fait |

---

## Ce que vous pouvez lui demander

```text
Vérifie mon VPS : disque, mémoire, services en échec — corrige ce qui est sûr.
Recherche X sur le web et produis un rapport PDF sourcé.
Surveille ce dossier et résume-moi les nouveaux documents sur Telegram.
Chaque matin à 8h : mon agenda, la météo, tout ce qui cloche dans les logs.
Connecte-toi en SSH au serveur de backup et vérifie que le job de cette nuit a bien tourné.
Mets-toi à jour.
```

Sous le capot, les outils intégrés sont sélectionnés sémantiquement afin que
seuls les schémas utiles atteignent le modèle. Captain prend aussi en charge
les skills gouvernés, les serveurs MCP, la délégation multi-agent, les
workflows, l'automatisation navigateur et les appels d'outils durables que
l'agent peut revisiter, annuler ou ordonner par dépendances.

---

## Documentation

| Guide | Contenu |
|---|---|
| [Getting Started](docs/getting-started.md) | Install → configuration → première conversation |
| [Configuration](docs/configuration.md) | `config.toml`, providers, modèles, toutes les options |
| [CLI Reference](docs/cli-reference.md) | Toutes les commandes et flags |
| [Providers](docs/providers.md) | Providers de modèles, auth, repli, routage |
| [Channel Adapters](docs/channel-adapters.md) | Configuration Telegram, Discord, Signal, Email |
| [Sécurité](docs/security.md) | Authentification, capacités, secrets, approbations et audit |
| [Built-in Tools](docs/captain-tools/) | Documentation des outils par famille |
| [Architecture](docs/architecture.md) | Crates, boucle d'agent, design du kernel |
| [API Reference](docs/api-reference.md) | Endpoints REST, auth, streaming |
| [VPS Deployment](docs/deployment/github-vps-install.md) | Installs headless, reverse proxy, HTTPS |
| [MCP](docs/captain-tools/mcp.md) | Serveurs d'outils externes et contrat de transport |
| [Troubleshooting](docs/troubleshooting.md) | Problèmes courants et leurs correctifs |
| [Notes de release 0.1.0-alpha.4](docs/releases/v0.1.0-alpha.4.md) | Corrections autoritaires, rappel actif complet et continuité CLI |
| [Docs Status (DOC2)](docs/DOCS_STATUS.md) | Contrats actuels, surfaces gelées et documents historiques |

> Les guides détaillés dans `docs/` sont actuellement en anglais uniquement.

---

## Posture sécurité

- L'API se lie à `127.0.0.1` par défaut et **refuse de démarrer** sur une
  interface publique sans authentification configurée.
- L'accès web/API exige une session connectée ou une clé API bearer ;
  l'éditeur de configuration web est authentifié.
- Les outils sensibles passent par le flux d'approbation ; les motifs shell
  hyper-critiques sont bloqués ou forcent une approbation ponctuelle quelle
  que soit la politique.
- Budgets par agent : tokens, coût horaire/quotidien/mensuel, fréquence
  d'appels d'outils.
- Détecteur de boucles : coupe-circuits sur répétition, ping-pong, et
  échecs consécutifs.
- Listes blanches de canaux refusées par défaut ; piste d'audit chaînée par
  hash ; les secrets vivent dans `secrets.env` ou le coffre chiffré, jamais
  dans les fichiers de configuration.

L'état vit sous `~/.captain/` — `config.toml` est la source de vérité
unique, rechargée à chaud à chaque changement.

---

## Développement

```bash
cargo test --workspace              # suite complète
cargo build --release -p captain-cli
scripts/release-readiness.sh         # gate locale complète de release
CAPTAIN_VERSION=vX.Y.Z scripts/release-all.sh  # les 5 cibles CLI en local
CAPTAIN_VERSION=vX.Y.Z scripts/publish-release-local.sh
docker build --build-arg CAPTAIN_BUILD_VERSION=vX.Y.Z -t captain:vX.Y.Z .
```

`release-all.sh` construit les deux bundles macOS, les deux bundles Linux et le
bundle CLI Windows ; le cross-build Windows utilise `cargo-xwin`, LLVM et NASM. Après une
gate release propre, `publish-release-local.sh` valide les 20 assets, pousse la
branche courante, construit et pousse l'image GHCR `linux/amd64` +
`linux/arm64`, puis publie le tag et la GitHub Release. L'image réutilise les
deux binaires Linux vérifiés au lieu de recompiler Captain sous émulation. Avant
l'assemblage de l'image, le publisher prépare depuis le cache Captain local du
mainteneur un snapshot FastEmbed verrouillé par checksum dans `dist/docker/`,
ignoré par Git. Il n'est ni committé ni ajouté aux 20 assets, et le build Docker
le vérifie de nouveau sans dépendre d'un CDN de modèles actif.
Authentifiez `gh` une fois avec
`gh auth refresh -h github.com -s read:packages,write:packages` ; ne passez pas
de token dans la ligne de commande. Le workflow release GitHub est
un secours manuel explicite et un push de tag ne le déclenche pas. La CI reste
disponible par déclenchement manuel explicite pour le formatage, Clippy strict,
les audits sécurité/secrets et les checks/tests du workspace.

---

## Licence

Double licence [MIT](LICENSE-MIT) ou [Apache 2.0](LICENSE-APACHE), à votre
choix.
