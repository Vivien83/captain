fn truncate_with_ellipsis(value: &str, max_chars: usize, take_chars: usize) -> String {
    if value.chars().count() > max_chars {
        format!("{}…", value.chars().take(take_chars).collect::<String>())
    } else {
        value.to_string()
    }
}

pub(crate) fn agent_lifecycle_line(kind: &str, agent_label: &str, detail: Option<&str>) -> String {
    let suffix = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    match kind {
        "terminated" => format!("↩︎ sous-agent {agent_label} terminé{suffix}"),
        "crashed" => format!("⚠ sous-agent {agent_label} a planté{suffix}"),
        other => format!("sous-agent {agent_label}: {other}{suffix}"),
    }
}

pub(crate) fn memory_stored_line(
    subject: &str,
    predicate: &str,
    object: &str,
    source: &str,
) -> String {
    let object = truncate_with_ellipsis(object, 80, 78);
    format!("🧠 mémorisé · {subject}/{predicate} = \"{object}\"   ({source})")
}

pub(crate) fn memory_queued_line(
    review_id: &str,
    subject: &str,
    predicate: &str,
    object: &str,
    source: &str,
) -> String {
    let object = truncate_with_ellipsis(object, 80, 78);
    format!("💭 apprentissage à valider · {subject}/{predicate} = \"{object}\"   ({source}, {review_id})")
}

pub(crate) fn skill_proposal_line(
    proposal_id: &str,
    name: &str,
    description: &str,
    trigger_hint: &str,
    confidence: f32,
    family: Option<&str>,
) -> String {
    let description = truncate_with_ellipsis(description, 100, 98);
    let family_label = match family {
        Some("software-development") => "développement",
        Some("project-management") => "projet",
        Some("review-release") => "review/release",
        Some("platform-devops") => "devops",
        Some("data-ai") => "data/ia",
        Some("product-design") => "produit/design",
        Some("business-tools") => "outils métier",
        Some("security-compliance") => "sécurité",
        _ => "automatisation",
    };
    let hint = if trigger_hint.is_empty() {
        String::new()
    } else {
        let trigger_hint =
            captain_runtime::skill_proposer::localize_trigger_hint(trigger_hint, "fr");
        format!(" · quand : {trigger_hint}")
    };
    let short_id = proposal_id.chars().take(8).collect::<String>();
    format!(
        "🛠️ skill proposé · {name} — {description}{hint} · famille: {family_label}   ({:.0}%, {short_id}) · /skills-proposed",
        confidence * 100.0
    )
}

#[cfg(test)]
#[path = "event_messages/tests.rs"]
mod tests;
