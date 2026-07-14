# SSH avec Captain — guide pratique

Captain peut piloter un serveur distant via SSH (commandes shell + transfert
SFTP) en restant aussi safe qu'un binaire `ssh` natif. Ce guide couvre
l'installation d'une clé, l'usage côté agent, les modes de vérification de
host key, et le troubleshooting des cas qu'on rencontre vraiment.

---

## Vue d'ensemble

| Phase | Rôle |
|---|---|
| **Q.6** | Vault SSH typé (`SshKey` AES-256-GCM dans `vault.enc`) + 5 commandes CLI `captain ssh add/list/test/remove/use` |
| **Q.7** | Tool `ssh_exec` exposé au LLM (russh, tokio-native, pas de shell out) |
| **Q.8** | Tools `ssh_upload` / `ssh_download` (russh-sftp) |
| **Q.7c** | Persistance `known_hosts` (TOFU au 1er connect, MITM detection ensuite) |
| **Q.7c.b** | CLI `captain ssh known-hosts list/clear/mode` + sidecar `~/.captain/ssh_kh_mode` |
| **Q.9** | Critical patterns (`rm -rf /`, `dd of=/dev/`, `DROP DATABASE`, fork bomb…) bloqués pré-envoi |

Toute l'infrastructure est embeddée — pas besoin de l'OpenSSH client ni
d'un agent SSH externe pour que Captain fonctionne.

---

## Workflow standard

⚠️ **Règle d'or** : la clé privée est générée **sur ta machine locale**
et n'en sort jamais. Tu copies seulement la clé **publique** sur le serveur
distant. Ne colle jamais une clé privée dans un chat, un ticket, un email.

### 1. Initialiser le vault (une fois par machine)

```bash
captain vault init
```

Crée `~/.captain/vault.enc` (AES-256-GCM). La master key est stockée dans
le keyring OS (macOS Keychain, Wincred, Secret Service Linux). Fallback
env `CAPTAIN_VAULT_KEY` pour les contextes headless / CI.

### 2. Générer une clé ed25519 sur ta machine

```bash
ssh-keygen -t ed25519 -C "$USER@$(hostname)-prod-server" -f ~/.ssh/id_ed25519_prod_server
```

`-C` = commentaire (utile pour identifier la clé dans `authorized_keys` plus
tard). Pas de passphrase = facile pour automation, mais tu peux en mettre
une — Captain la stocke dans le vault chiffré.

### 3. Pousser la clé publique sur le VPS

```bash
ssh-copy-id -i ~/.ssh/id_ed25519_prod_server.pub root@server.example.com
```

Si `ssh-copy-id` n'est pas dispo (Windows) ou si tu n'as pas encore d'accès
SSH au VPS, utilise la console web du fournisseur pour ajouter manuellement
la ligne dans `~/.ssh/authorized_keys`.

⚠️ **Piège fréquent** : `authorized_keys` doit avoir **une clé par ligne**.
Si tu utilises `echo '...' >> authorized_keys` et que le fichier ne se
termine pas par `\n`, tu colles 2 clés sur la même ligne → sshd refuse
les deux. Mieux : `nano` ou `cat >> authorized_keys <<'EOF' … EOF`.

### 4. Tester depuis ta machine

```bash
ssh -i ~/.ssh/id_ed25519_prod_server root@server.example.com 'hostname && id'
```

Si ça marche, Captain marchera aussi. Si ça échoue, voir [troubleshooting](#troubleshooting).

### 5. Enregistrer la clé dans le vault Captain

```bash
captain ssh add prod-server
# Path to private key file: ~/.ssh/id_ed25519_prod_server
# Passphrase (Enter if none):
# Host: server.example.com
# User: root
# Port [22]: 22
```

Captain calcule le SHA-256 fingerprint de la clé (identique à
`ssh-keygen -lf`) et chiffre l'entrée sous la clé `ssh:prod-server` du vault.

### 6. Lister + tester

```bash
captain ssh list
# NAME       HOST              USER  PORT  FINGERPRINT
# prod-server server.example.com root 22    SHA256:abc…

captain ssh test prod-server
# Connecting to server.example.com:22… OK
# ✔ TCP reachable.
```

### 7. Marquer comme défaut (optionnel)

```bash
captain ssh use prod-server
```

---

## Usage côté agent

Une fois la clé enregistrée, Captain peut l'utiliser via 3 tools :

```jsonc
// Exécution distante
{
  "tool": "ssh_exec",
  "input": {
    "key_name": "prod-server",
    "command": "systemctl status nginx",
    "timeout_secs": 30
  }
}

// Upload
{
  "tool": "ssh_upload",
  "input": {
    "key_name": "prod-server",
    "local_path": "/local/nginx.conf",
    "remote_path": "/etc/nginx/conf.d/app.conf"
  }
}

// Download
{
  "tool": "ssh_download",
  "input": {
    "key_name": "prod-server",
    "remote_path": "/var/log/nginx/error.log",
    "local_path": "./error.log"
  }
}
```

Le LLM choisit ces tools de lui-même quand tu demandes en langage naturel
(« redémarre nginx sur prod-server », « récupère le dernier log d'erreur »,
etc.). Pour forcer un appel précis, demande explicitement le tool et les
paramètres dans le prompt.

---

## Vérification de host key (Q.7c)

Captain stocke les host keys distantes dans `~/.captain/known_hosts`
(format OpenSSH standard, lisible avec `cat`, diffable, rotatable). 3 modes :

| Mode | Comportement |
|---|---|
| **`tofu_learn`** *(défaut)* | 1er connect = trust silencieux + apprentissage. 2e connect+ = strict (refuse si la clé a changé → MITM detection). |
| **`strict`** | Refuse tout host inconnu. Bon pour CI / prod-paranoid. |
| **`insecure`** | Accepte tout (legacy). Ne jamais utiliser en prod. |

### Commandes

```bash
# Voir le mode actif
captain ssh known-hosts mode

# Activer strict (refuse tout host pas encore connu)
captain ssh known-hosts mode strict

# Lister les host keys stockées
captain ssh known-hosts list

# Vider le store (backup .bak.<ts> conservé)
captain ssh known-hosts clear
```

### Override one-shot via env

```bash
CAPTAIN_SSH_KH_MODE=strict captain start
```

L'env var prime sur le sidecar `~/.captain/ssh_kh_mode`. Pratique pour
lancer un job CI temporaire en mode parano sans toucher à la conf
permanente.

### Quand le mismatch (MITM) se déclenche

Si la clé du serveur a changé entre 2 connexions, Captain refuse la
handshake AVANT d'envoyer la commande :

```
WARN ssh: host key REFUSED host=server.example.com port=22
  reason=Host key VERIFICATION FAILED for server.example.com:22:
  The server key changed at line 1.
  Possible man-in-the-middle attack — refused.
  Inspect ~/.captain/known_hosts and remove the offending line
  if you trust the change.
```

Le tool retourne `Failed to connect: Unknown server key` à l'agent. Aucun
octet n'est transmis au serveur potentiellement malveillant.

Si c'est un changement légitime (rotation de host key, réinstall serveur),
édite `~/.captain/known_hosts` à la main pour supprimer la vieille ligne,
ou fais `captain ssh known-hosts clear` puis reconnecte (re-learn TOFU).

---

## Garde-fous critiques (Q.9)

Indépendamment du mode known_hosts, Captain bloque toujours les patterns
hyper-critiques **avant** envoi distant (couche supplémentaire — on ne fait
pas confiance au serveur pour se protéger lui-même) :

- `rm -rf /`, `rm -rf /*`, `rm -rf ~`, `rm -rf $HOME`
- `dd if=` ou `dd of=/dev/`
- `mkfs`, `wipefs`
- `DROP DATABASE`, `DROP SCHEMA`, `TRUNCATE TABLE`
- `:(){ :|:&};:` (fork bomb)
- `chmod -R 777 /`
- `git push --force origin main` / `master`

En mode `open` (défaut Captain), ces patterns déclenchent un modal
d'approbation Captain (4 choix : approve once / session / always /
reject). En mode `safe` ils sont bloqués direct sans prompt. En mode
`paranoid` toute commande shell distante demande approbation.

Tout autre `rm`, `apt-get`, `systemctl restart`, modification de fichier,
etc. **passe sans demande** — Captain est conçu pour faire ce que tu lui
demandes, pas pour t'interrompre toutes les 30 secondes.

---

## Audit log

Chaque opération SSH est journalisée dans `~/.captain/audit/ssh.log`
(JSONL append-only) :

```bash
tail -5 ~/.captain/audit/ssh.log | jq .
```

```jsonc
{ "ts": 1777226939, "op": "add",      "key": "prod-server", "ok": true,  "detail": "SHA256:abc…" }
{ "ts": 1777226946, "op": "test",     "key": "prod-server", "ok": true,  "detail": "tcp ok" }
{ "ts": 1777227139, "op": "exec",     "key": "prod-server", "ok": true,  "detail": "hostname && uname -srm (287ms)" }
{ "ts": 1777227707, "op": "upload",   "key": "prod-server", "ok": true,  "detail": "/local -> /remote (335ms)" }
{ "ts": 1777227895, "op": "exec",     "key": "prod-server", "ok": false, "detail": "echo … (112ms)" }
```

`ok: false` signale les refus (MITM, timeout, auth failed, critical pattern bloqué).

---

## Troubleshooting

### `Connection refused` ou timeout sur le port 22

1. **Le VPS est joignable** ? `ping <ip>` (ICMP n'est pas SSH mais teste le réseau)
2. **Le port SSH est-il standard** ? `nc -zv <ip> 22 ; nc -zv <ip> 2222`
3. **Firewall corporate** ? Si tu es dans un réseau d'entreprise, l'outbound
   TCP 22 vers IPs externes est souvent filtré. Vérifie depuis un autre
   réseau (4G téléphone) ou contacte ton IT.
4. **Firewall fournisseur** (Hetzner Cloud Firewall, OVH, etc.) ? Vérifie
   les règles côté panel.

### `Permission denied (publickey)`

1. **Pub key bien dans `authorized_keys`** ? `ssh user@server.example.com 'tail -2 ~/.ssh/authorized_keys'`
2. **Une clé par ligne** ? `wc -l ~/.ssh/authorized_keys` (et inspect avec `cat`)
3. **Permissions correctes** ? `~/.ssh/` doit être `drwx------` (700),
   `authorized_keys` doit être `-rw-------` (600). sshd refuse si trop laxe.
4. **`PermitRootLogin yes`** dans `/etc/ssh/sshd_config` si tu te connectes en root
5. **Logs côté serveur** : `tail -20 /var/log/auth.log` ou `journalctl -u ssh -n 30`

### `fail2ban` t'a banni

Si tu as enchaîné plusieurs tentatives ratées, fail2ban a probablement
banni ton IP. Sur le serveur :

```bash
sudo fail2ban-client status sshd            # voir bannis
sudo fail2ban-client set sshd unbanip <ip>  # libérer
sudo fail2ban-client set sshd addignoreip <ip>  # whitelist permanente
```

### Ma clé privée a été partagée par accident

Considère-la **compromise** quel que soit le canal. Procédure :

```bash
# Sur le serveur — retirer la pub key correspondante
ssh user@server.example.com
sed -i '/<comment-ou-fingerprint>/d' ~/.ssh/authorized_keys
exit

# Sur ta machine — supprimer la priv
rm ~/.ssh/<nom_clé> ~/.ssh/<nom_clé>.pub

# Dans Captain — supprimer du vault
captain ssh remove <alias>

# Régénérer une nouvelle paire et recommencer le workflow
```

### `Vault not initialized`

```bash
captain vault init
```

Si déjà init mais inaccessible (key OS keyring perdue) : c'est la fin du
vault. Backup : `cp ~/.captain/vault.enc ~/.captain/vault.enc.bak`,
réinitialise et ré-enregistre les clés.

---

## Cleanup d'une clé

```bash
# Côté local
captain ssh remove <alias>     # confirme y/N, supprime du vault
rm ~/.ssh/id_ed25519_<alias>{,.pub}

# Côté serveur
ssh <existing-other-access>
sed -i '/<comment-de-la-clé>/d' ~/.ssh/authorized_keys

# Optionnel : forget host key locale
captain ssh known-hosts clear
# Ou édite ~/.captain/known_hosts à la main pour ne retirer qu'une entrée
```

---

## Variables d'environnement

| Var | Effet |
|---|---|
| `CAPTAIN_HOME` | Override `~/.captain/` (vault, known_hosts, audit, sidecar mode) |
| `CAPTAIN_VAULT_KEY` | Master key vault en clair (headless / CI seulement) |
| `CAPTAIN_KNOWN_HOSTS` | Override path complet du fichier known_hosts |
| `CAPTAIN_SSH_KH_MODE` | Override one-shot du mode (`tofu_learn` / `strict` / `insecure`) |

---

## Limites actuelles

- Pas de support pour les jump hosts (`ProxyJump`)
- Pas de support pour SSH agent forwarding
- SFTP transfert blocking (whole file en mémoire) — convient aux
  fichiers de config / scripts ; pour gros transferts utiliser
  `ssh_exec` + `rsync`
- known_hosts ne gère pas les algorithmes ECDSA (ed25519 et rsa
  uniquement par construction du `ssh-key` crate features)
