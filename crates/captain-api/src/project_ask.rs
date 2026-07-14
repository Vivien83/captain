use dashmap::DashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const PROJECT_ASK_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
pub struct ProjectAskRegistration {
    pub project_id: String,
    pub project_slug: String,
    pub project_name: String,
    pub phase: String,
    pub worker_role: String,
    pub question: String,
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ProjectAskAnswerReceipt {
    pub ask_id: String,
    pub meta: ProjectAskRegistration,
    pub answer: String,
}

struct PendingProjectAsk {
    meta: ProjectAskRegistration,
    tx: mpsc::Sender<String>,
    created_at: Instant,
}

static PENDING_PROJECT_ASKS: LazyLock<DashMap<String, PendingProjectAsk>> =
    LazyLock::new(DashMap::new);

pub fn register_project_ask(meta: ProjectAskRegistration, tx: mpsc::Sender<String>) -> String {
    prune_expired();
    let ask_id = uuid::Uuid::new_v4().to_string();
    PENDING_PROJECT_ASKS.insert(
        ask_id.clone(),
        PendingProjectAsk {
            meta,
            tx,
            created_at: Instant::now(),
        },
    );
    ask_id
}

pub async fn answer_project_ask(ask_id_or_prefix: &str, answer: &str) -> Result<String, String> {
    let receipt = answer_project_ask_with_receipt(ask_id_or_prefix, answer).await?;
    Ok(format!(
        "✅ Réponse envoyée au projet « {} » [{}] : {}",
        receipt.meta.project_name,
        short_id(&receipt.ask_id),
        receipt.answer
    ))
}

pub async fn answer_project_ask_with_receipt(
    ask_id_or_prefix: &str,
    answer: &str,
) -> Result<ProjectAskAnswerReceipt, String> {
    prune_expired();
    answer_project_ask_resolved(resolve_unique_key(ask_id_or_prefix)?, answer).await
}

pub async fn answer_project_ask_for_project_with_receipt(
    project_id: &str,
    ask_id_or_prefix: &str,
    answer: &str,
) -> Result<ProjectAskAnswerReceipt, String> {
    prune_expired();
    answer_project_ask_resolved(
        resolve_unique_project_key(project_id, ask_id_or_prefix)?,
        answer,
    )
    .await
}

async fn answer_project_ask_resolved(
    key: String,
    answer: &str,
) -> Result<ProjectAskAnswerReceipt, String> {
    prune_expired();
    let Some((ask_id, pending)) = PENDING_PROJECT_ASKS.remove(&key) else {
        return Err(format!(
            "La question projet [{}] n'est plus active.",
            short_id(&key)
        ));
    };
    let answer = normalize_project_ask_answer(answer, pending.meta.options.as_deref())?;
    pending.tx.send(answer.clone()).await.map_err(|_| {
        format!(
            "La question projet [{}] n'est plus active. Relance ou reprends le projet.",
            short_id(&ask_id)
        )
    })?;
    Ok(ProjectAskAnswerReceipt {
        ask_id,
        meta: pending.meta,
        answer,
    })
}

pub fn expire_project_asks_for_run(project_id: &str) {
    let keys: Vec<String> = PENDING_PROJECT_ASKS
        .iter()
        .filter(|entry| entry.meta.project_id == project_id)
        .map(|entry| entry.key().clone())
        .collect();
    for key in keys {
        PENDING_PROJECT_ASKS.remove(&key);
    }
}

fn resolve_unique_key(ask_id_or_prefix: &str) -> Result<String, String> {
    let prefix = ask_id_or_prefix.trim();
    if prefix.is_empty() {
        return Err("ID de question projet manquant.".to_string());
    }
    let matches: Vec<String> = PENDING_PROJECT_ASKS
        .iter()
        .filter(|entry| entry.key().starts_with(prefix))
        .map(|entry| entry.key().clone())
        .collect();
    match matches.len() {
        0 => Err(format!(
            "Aucune question projet active ne correspond à « {prefix} »."
        )),
        1 => Ok(matches[0].clone()),
        n => Err(format!(
            "{n} questions projet correspondent à « {prefix} ». Utilise plus de caractères de l'ID."
        )),
    }
}

fn resolve_unique_project_key(project_id: &str, ask_id_or_prefix: &str) -> Result<String, String> {
    let prefix = ask_id_or_prefix.trim();
    if prefix.is_empty() {
        return Err("ID de question projet manquant.".to_string());
    }
    let matches: Vec<String> = PENDING_PROJECT_ASKS
        .iter()
        .filter(|entry| entry.meta.project_id == project_id && entry.key().starts_with(prefix))
        .map(|entry| entry.key().clone())
        .collect();
    match matches.len() {
        0 => Err(format!(
            "Aucune question projet active ne correspond à « {prefix} » pour ce projet."
        )),
        1 => Ok(matches[0].clone()),
        n => Err(format!(
            "{n} questions projet correspondent à « {prefix} » pour ce projet. Utilise plus de caractères de l'ID."
        )),
    }
}

pub fn normalize_project_ask_answer(
    answer: &str,
    options: Option<&[String]>,
) -> Result<String, String> {
    let trimmed = answer.trim();
    if trimmed.is_empty() {
        return Err("Réponse vide.".to_string());
    }
    if let Some(options) = options {
        if let Some(raw_idx) = trimmed.strip_prefix("@idx:") {
            let idx = raw_idx
                .parse::<usize>()
                .map_err(|_| "Index de choix projet invalide.".to_string())?;
            return options
                .get(idx)
                .cloned()
                .ok_or_else(|| "Choix projet expiré ou invalide.".to_string());
        }
        if let Ok(idx) = trimmed.parse::<usize>() {
            if idx > 0 {
                if let Some(option) = options.get(idx - 1) {
                    return Ok(option.clone());
                }
            }
        }
    }
    Ok(trimmed.to_string())
}

fn prune_expired() {
    let keys: Vec<String> = PENDING_PROJECT_ASKS
        .iter()
        .filter(|entry| entry.created_at.elapsed() > PROJECT_ASK_TTL)
        .map(|entry| entry.key().clone())
        .collect();
    for key in keys {
        PENDING_PROJECT_ASKS.remove(&key);
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn project_ask_maps_callback_index_to_option() {
        let (tx, mut rx) = mpsc::channel(1);
        let ask_id = register_project_ask(
            ProjectAskRegistration {
                project_id: "p1".to_string(),
                project_slug: "demo".to_string(),
                project_name: "Demo".to_string(),
                phase: "plan".to_string(),
                worker_role: "planner".to_string(),
                question: "Choisir ?".to_string(),
                options: Some(vec!["Option A".to_string(), "Option B".to_string()]),
            },
            tx,
        );

        let msg = answer_project_ask(&ask_id, "@idx:1").await.unwrap();
        assert!(msg.contains("Demo"));
        assert_eq!(rx.recv().await.unwrap(), "Option B");
    }

    #[tokio::test]
    async fn project_ask_maps_human_number_to_option() {
        let (tx, mut rx) = mpsc::channel(1);
        let ask_id = register_project_ask(
            ProjectAskRegistration {
                project_id: "p1b".to_string(),
                project_slug: "demo".to_string(),
                project_name: "Demo".to_string(),
                phase: "plan".to_string(),
                worker_role: "planner".to_string(),
                question: "Choisir ?".to_string(),
                options: Some(vec!["Option A".to_string(), "Option B".to_string()]),
            },
            tx,
        );

        answer_project_ask(&ask_id, "1").await.unwrap();
        assert_eq!(rx.recv().await.unwrap(), "Option A");
    }

    #[tokio::test]
    async fn project_ask_accepts_free_text() {
        let (tx, mut rx) = mpsc::channel(1);
        let ask_id = register_project_ask(
            ProjectAskRegistration {
                project_id: "p2".to_string(),
                project_slug: "demo".to_string(),
                project_name: "Demo".to_string(),
                phase: "build".to_string(),
                worker_role: "builder".to_string(),
                question: "Besoin de précision".to_string(),
                options: None,
            },
            tx,
        );

        answer_project_ask(&ask_id[..8], "Continue avec FastAPI")
            .await
            .unwrap();
        assert_eq!(rx.recv().await.unwrap(), "Continue avec FastAPI");
    }

    #[tokio::test]
    async fn project_ask_project_scoped_answer_rejects_other_project() {
        let (tx, mut rx) = mpsc::channel(1);
        let ask_id = register_project_ask(
            ProjectAskRegistration {
                project_id: "project-a".to_string(),
                project_slug: "demo".to_string(),
                project_name: "Demo".to_string(),
                phase: "build".to_string(),
                worker_role: "builder".to_string(),
                question: "Besoin de précision".to_string(),
                options: None,
            },
            tx,
        );

        let err = answer_project_ask_for_project_with_receipt("project-b", &ask_id, "Nope")
            .await
            .unwrap_err();
        assert!(err.contains("pour ce projet"));

        let receipt = answer_project_ask_for_project_with_receipt(
            "project-a",
            &ask_id[..8],
            "Continue avec FastAPI",
        )
        .await
        .unwrap();
        assert_eq!(receipt.meta.project_id, "project-a");
        assert_eq!(rx.recv().await.unwrap(), "Continue avec FastAPI");
    }
}
