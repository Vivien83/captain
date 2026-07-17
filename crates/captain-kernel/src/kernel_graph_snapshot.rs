use captain_types::memory::{GraphMatch, GraphPattern, Memory, RelationType};
use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;

use super::CaptainKernel;

impl CaptainKernel {
    /// Generate a compact markdown snapshot of the knowledge graph for prompt injection.
    /// Writes GRAPH.md into `home_dir` so all agents can read it as a global context file.
    pub(crate) async fn generate_graph_snapshot(&self) {
        let mut out = graph_snapshot_header();

        let all_pattern = GraphPattern {
            source: None,
            relation: None,
            target: None,
            max_depth: 1,
        };
        if let Ok(matches) = self.memory.query_graph(all_pattern).await {
            let sections = group_graph_matches(&matches);
            append_graph_sections(&mut out, &sections);
        }

        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z");
        let _ = writeln!(out, "\n> Last updated: {now}");

        let graph_path = self.config.home_dir.join("GRAPH.md");
        if let Err(e) = captain_types::durable_fs::atomic_write(&graph_path, out.as_bytes()) {
            tracing::warn!("Failed to write GRAPH.md: {e}");
        } else {
            tracing::debug!(path = %graph_path.display(), "GRAPH.md snapshot written");
        }
    }
}

fn graph_snapshot_header() -> String {
    String::from("# Graph Snapshot\n> Auto-generated — do not edit manually\n\n")
}

fn group_graph_matches(matches: &[GraphMatch]) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for m in matches.iter().take(200) {
        let section = section_label(&m.source.id, &m.relation.relation);
        let line = graph_match_line(m);
        sections.entry(section).or_default().push(line);
    }

    sections
}

fn append_graph_sections(out: &mut String, sections: &BTreeMap<String, Vec<String>>) {
    let mut line_count = 0usize;
    for (section, lines) in sections {
        if line_count >= 100 {
            let _ = writeln!(out, "\n> (output capped at 100 lines)");
            break;
        }
        let _ = writeln!(out, "## {section}");
        for line in lines {
            if line_count >= 100 {
                break;
            }
            out.push_str(line);
            out.push('\n');
            line_count += 1;
        }
        out.push('\n');
    }
}

fn graph_match_line(m: &GraphMatch) -> String {
    format!(
        "- {} → {} → {}",
        m.source.name,
        rel_label(&m.relation.relation),
        m.target.name
    )
}

fn section_label(source_id: &str, relation: &RelationType) -> String {
    if source_id.starts_with('_') {
        let prefix = source_id
            .trim_start_matches('_')
            .split("::")
            .next()
            .unwrap_or("other");
        return capitalize_ascii(prefix);
    }

    rel_label(relation)
        .split('_')
        .map(capitalize_ascii)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_ascii(value: &str) -> String {
    let mut out = value.to_string();
    if let Some(c) = out.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    out
}

fn rel_label(r: &RelationType) -> String {
    match r {
        RelationType::WorksAt => "works_at".to_string(),
        RelationType::KnowsAbout => "knows_about".to_string(),
        RelationType::RelatedTo => "related_to".to_string(),
        RelationType::DependsOn => "depends_on".to_string(),
        RelationType::OwnedBy => "owned_by".to_string(),
        RelationType::CreatedBy => "created_by".to_string(),
        RelationType::LocatedIn => "located_in".to_string(),
        RelationType::PartOf => "part_of".to_string(),
        RelationType::Uses => "uses".to_string(),
        RelationType::Produces => "produces".to_string(),
        RelationType::Custom(s) => s.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::memory::{Entity, EntityType, Relation};
    use std::collections::HashMap;

    fn entity(id: &str, name: &str) -> Entity {
        let now = chrono::Utc::now();
        Entity {
            id: id.to_string(),
            entity_type: EntityType::Concept,
            name: name.to_string(),
            properties: HashMap::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn graph_match(source_id: &str, relation: RelationType, target_id: &str) -> GraphMatch {
        GraphMatch {
            source: entity(source_id, "Source"),
            relation: Relation {
                source: source_id.to_string(),
                relation,
                target: target_id.to_string(),
                properties: HashMap::new(),
                confidence: 1.0,
                created_at: chrono::Utc::now(),
            },
            target: entity(target_id, "Target"),
        }
    }

    #[test]
    fn relation_labels_match_snapshot_contract() {
        assert_eq!(rel_label(&RelationType::WorksAt), "works_at");
        assert_eq!(rel_label(&RelationType::KnowsAbout), "knows_about");
        assert_eq!(rel_label(&RelationType::RelatedTo), "related_to");
        assert_eq!(rel_label(&RelationType::DependsOn), "depends_on");
        assert_eq!(rel_label(&RelationType::OwnedBy), "owned_by");
        assert_eq!(rel_label(&RelationType::CreatedBy), "created_by");
        assert_eq!(rel_label(&RelationType::LocatedIn), "located_in");
        assert_eq!(rel_label(&RelationType::PartOf), "part_of");
        assert_eq!(rel_label(&RelationType::Uses), "uses");
        assert_eq!(rel_label(&RelationType::Produces), "produces");
        assert_eq!(
            rel_label(&RelationType::Custom("supports".to_string())),
            "supports"
        );
    }

    #[test]
    fn section_label_prefers_source_prefix_then_relation_words() {
        assert_eq!(
            section_label("_user::vivien", &RelationType::WorksAt),
            "User"
        );
        assert_eq!(
            section_label("_family::home", &RelationType::Uses),
            "Family"
        );
        assert_eq!(
            section_label("project:captain", &RelationType::DependsOn),
            "Depends On"
        );
    }

    #[test]
    fn graph_matches_are_grouped_and_capped_like_hermes() {
        let mut matches = Vec::new();
        for idx in 0..205 {
            matches.push(graph_match(
                &format!("project:{idx}"),
                RelationType::DependsOn,
                &format!("target:{idx}"),
            ));
        }

        let sections = group_graph_matches(&matches);
        let lines = sections.get("Depends On").expect("section exists");

        assert_eq!(lines.len(), 200);
    }

    #[test]
    fn appending_sections_caps_output_at_one_hundred_lines() {
        let mut sections = BTreeMap::new();
        sections.insert(
            "A".to_string(),
            (0..100).map(|idx| format!("- line {idx}")).collect(),
        );
        sections.insert("B".to_string(), vec!["- hidden".to_string()]);
        let mut out = graph_snapshot_header();

        append_graph_sections(&mut out, &sections);

        assert!(out.contains("## A"));
        assert!(!out.contains("## B"));
        assert!(out.contains("> (output capped at 100 lines)"));
    }
}
