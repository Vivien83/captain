//! Static SSH tool definitions.

use captain_types::tool::ToolDefinition;

pub fn ssh_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "ssh_upload".to_string(),
            description: "Téléverse un fichier local vers un hôte distant via SFTP (russh-sftp, embedded). La clé est résolue par alias depuis le vault Captain. Lecture/écriture en mémoire (pas de streaming) — convient aux fichiers de config, scripts, snippets de petite taille. Pour de gros transferts, utiliser shell_exec + rsync. Timeout par défaut 120s.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key_name": { "type": "string", "description": "Alias de la clé SSH stockée dans le vault" },
                    "local_path": { "type": "string", "description": "Chemin du fichier local à téléverser" },
                    "remote_path": { "type": "string", "description": "Chemin de destination sur l'hôte distant" },
                    "timeout_secs": { "type": "integer", "description": "Timeout total en secondes (défaut 120)" }
                },
                "required": ["key_name", "local_path", "remote_path"]
            }),
        },
        ToolDefinition {
            name: "ssh_download".to_string(),
            description: "Télécharge un fichier distant vers le poste local via SFTP (russh-sftp, embedded). La clé est résolue par alias depuis le vault. Lecture/écriture en mémoire (pas de streaming). Crée automatiquement les dossiers parents du chemin local si besoin. Timeout par défaut 120s.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key_name": { "type": "string", "description": "Alias de la clé SSH stockée dans le vault" },
                    "remote_path": { "type": "string", "description": "Chemin du fichier distant à télécharger" },
                    "local_path": { "type": "string", "description": "Chemin de destination local (parents créés au besoin)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout total en secondes (défaut 120)" }
                },
                "required": ["key_name", "remote_path", "local_path"]
            }),
        },
        ToolDefinition {
            name: "ssh_health_check".to_string(),
            description: "[SSH SANTE GROUPEE] Vérifie l'état d'un serveur ou d'un service via SSH avec une commande générée par Captain: host, uptime, disque, mémoire, CPU/load, services failed, Docker, ports, logs critiques ou logs service. À préférer à ssh_exec pour les checks de santé afin d'éviter de recomposer un gros script shell et de réduire les appels.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key_name": { "type": "string", "description": "Alias SSH du vault Captain." },
                    "service": { "type": "string", "description": "Service systemd optionnel à auditer (nom strictement validé)." },
                    "include_docker": { "type": "boolean", "default": true },
                    "include_ports": { "type": "boolean", "default": true },
                    "include_logs": { "type": "boolean", "default": true },
                    "log_lines": { "type": "integer", "description": "Lignes de logs, défaut 80, max 200." },
                    "timeout_secs": { "type": "integer", "description": "Timeout, défaut 60, max 180." }
                },
                "required": ["key_name"]
            }),
        },
        ToolDefinition {
            name: "ssh_exec".to_string(),
            description: "[ACCÈS DISTANT] Exécute une commande shell sur un hôte distant via SSH (russh, embedded — pas de shell out vers `ssh`). À utiliser SPONTANÉMENT — sans que l'utilisateur ne te précise l'IP/host — dès qu'il mentionne : (1) un alias familier (ex: 'prod-server', 'mon serveur', 'la machine X'), (2) une vérification d'état distante ('le serveur va bien ?', 'check le disque'), (3) une exécution sur une cible nommée. Pour les diagnostics santé/service, préférer d'abord ssh_health_check ou des snapshots finis (`systemctl status --no-pager`, `journalctl -n ... --no-pager`, `docker ps`, `docker logs --tail N` sans `-f`). La clé est résolue par alias depuis le vault Captain (cf. `captain ssh add`) ; essaye D'ABORD avec l'alias avant de demander quoi que ce soit à l'utilisateur. Si l'alias est générique ou incomplet, Captain tente une résolution non ambiguë ou la clé SSH par défaut. Les patterns hyper-critiques et les flux distants non bornés (`journalctl -f`, `docker logs -f`, `tail -f`, `watch`, `pm2 logs`, `docker stats` sans `--no-stream`) sont vérifiés AVANT l'envoi. Retourne stdout/stderr/exit code formatés. Timeout par défaut 60s. Si timeout_secs est explicitement défini, c'est une fenêtre de réévaluation bornée: Captain émet du progrès, renouvelle quelques fenêtres puis applique un plafond dur avec sortie partielle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key_name": { "type": "string", "description": "Alias de la clé SSH stockée dans le vault (ex: 'prod-server')" },
                    "command": { "type": "string", "description": "Commande à exécuter sur l'hôte distant" },
                    "timeout_secs": { "type": "integer", "description": "Fenêtre de réévaluation bornée en secondes (max 7200) quand elle est explicite; défaut sans paramètre explicite: garde-fou dur de 60s." }
                },
                "required": ["key_name", "command"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_exec_description_guides_bounded_remote_snapshots() {
        let tools = ssh_tool_definitions();
        let ssh_exec = tools
            .iter()
            .find(|tool| tool.name == "ssh_exec")
            .expect("ssh_exec definition should exist");

        assert!(ssh_exec.description.contains("ssh_health_check"));
        assert!(ssh_exec.description.contains("snapshots finis"));
        assert!(ssh_exec.description.contains("docker logs -f"));
        assert!(ssh_exec.description.contains("plafond dur"));
        assert!(ssh_exec.description.contains("sortie partielle"));
        assert_eq!(
            ssh_exec.input_schema["properties"]["timeout_secs"]["description"]
                .as_str()
                .unwrap(),
            "Fenêtre de réévaluation bornée en secondes (max 7200) quand elle est explicite; défaut sans paramètre explicite: garde-fou dur de 60s."
        );
    }
}
