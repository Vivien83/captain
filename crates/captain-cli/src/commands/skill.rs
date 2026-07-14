use std::path::{Path, PathBuf};

use crate::{captain_home, copy_dir_recursive, prompt_input};

pub(crate) fn cmd_skill_install(source: &str) {
    let home = captain_home();
    let skills_dir = home.join("skills");
    std::fs::create_dir_all(&skills_dir).unwrap_or_else(|e| {
        eprintln!("Error creating skills directory: {e}");
        std::process::exit(1);
    });

    let source_path = PathBuf::from(source);
    if source_path.exists() && source_path.is_dir() {
        install_local_skill(source, &source_path, &skills_dir);
    } else {
        install_marketplace_skill(source, &skills_dir);
    }
}

fn install_local_skill(source: &str, source_path: &PathBuf, skills_dir: &Path) {
    let manifest_path = source_path.join("skill.toml");
    if !manifest_path.exists() {
        install_openclaw_skill_or_exit(source, source_path, skills_dir);
        return;
    }

    let toml_str = std::fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        eprintln!("Error reading skill.toml: {e}");
        std::process::exit(1);
    });
    let manifest: captain_skills::SkillManifest = toml::from_str(&toml_str).unwrap_or_else(|e| {
        eprintln!("Error parsing skill.toml: {e}");
        std::process::exit(1);
    });

    let dest = skills_dir.join(&manifest.skill.name);
    copy_dir_recursive(source_path, &dest);
    println!(
        "Installed skill: {} v{}",
        manifest.skill.name, manifest.skill.version
    );
}

fn install_openclaw_skill_or_exit(source: &str, source_path: &PathBuf, skills_dir: &Path) {
    if !captain_skills::openclaw_compat::detect_openclaw_skill(source_path) {
        eprintln!("No skill.toml found in {source}");
        std::process::exit(1);
    }

    println!("Detected OpenClaw skill format. Converting...");
    match captain_skills::openclaw_compat::convert_openclaw_skill(source_path) {
        Ok(manifest) => {
            let dest = skills_dir.join(&manifest.skill.name);
            copy_dir_recursive(source_path, &dest);
            if let Err(e) =
                captain_skills::openclaw_compat::write_captain_manifest(&dest, &manifest)
            {
                eprintln!("Failed to write manifest: {e}");
                std::process::exit(1);
            }
            println!("Installed OpenClaw skill: {}", manifest.skill.name);
        }
        Err(e) => {
            eprintln!("Failed to convert OpenClaw skill: {e}");
            std::process::exit(1);
        }
    }
}

fn install_marketplace_skill(source: &str, skills_dir: &Path) {
    println!("Installing {source} from Captain Marketplace...");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let client = captain_skills::marketplace::MarketplaceClient::new(
        captain_skills::marketplace::MarketplaceConfig::default(),
    );
    match rt.block_on(client.install(source, skills_dir)) {
        Ok(version) => println!("Installed {source} {version}"),
        Err(e) => {
            eprintln!("Failed to install skill: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_list() {
    let home = captain_home();
    let skills_dir = home.join("skills");

    let mut registry = captain_skills::registry::SkillRegistry::new(skills_dir);
    match registry.load_all() {
        Ok(0) => println!("No skills installed."),
        Ok(count) => {
            println!("{count} skill(s) installed:\n");
            println!(
                "{:<20} {:<10} {:<8} DESCRIPTION",
                "NAME", "VERSION", "TOOLS"
            );
            println!("{}", "-".repeat(70));
            for skill in registry.list() {
                println!(
                    "{:<20} {:<10} {:<8} {}",
                    skill.manifest.skill.name,
                    skill.manifest.skill.version,
                    skill.manifest.tools.provided.len(),
                    skill.manifest.skill.description,
                );
            }
        }
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_skill_doc(out: Option<PathBuf>) {
    let home = captain_home();
    let skills_dir = home.join("skills");
    let mut registry = captain_skills::registry::SkillRegistry::new(skills_dir);
    let count = match registry.load_all() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("Error loading skills: {e}");
            std::process::exit(1);
        }
    };

    let md = render_skill_doc(&registry, count);
    match out {
        Some(path) => match std::fs::write(&path, &md) {
            Ok(()) => println!("Wrote {} ({} bytes)", path.display(), md.len()),
            Err(e) => {
                eprintln!("Failed to write {}: {e}", path.display());
                std::process::exit(1);
            }
        },
        None => print!("{md}"),
    }
}

fn render_skill_doc(registry: &captain_skills::registry::SkillRegistry, count: usize) -> String {
    let mut md = String::new();
    md.push_str("# Captain — Skills Reference\n\n");
    md.push_str(&format!(
        "> Auto-generated by `captain skill doc`. {count} skill(s) installed.\n\n"
    ));

    if count == 0 {
        md.push_str("_No skills installed._\n");
        return md;
    }

    md.push_str("| Name | Version | Tools | Description |\n|---|---|---|---|\n");
    for skill in registry.list() {
        md.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            skill.manifest.skill.name,
            skill.manifest.skill.version,
            skill.manifest.tools.provided.len(),
            skill.manifest.skill.description.replace('|', "\\|"),
        ));
    }
    md.push('\n');
    for skill in registry.list() {
        md.push_str(&format!(
            "## `{}` v{}\n\n",
            skill.manifest.skill.name, skill.manifest.skill.version
        ));
        md.push_str(&format!("{}\n\n", skill.manifest.skill.description));
        if !skill.manifest.skill.author.is_empty() {
            md.push_str(&format!("- **Author:** {}\n", skill.manifest.skill.author));
        }
        if !skill.manifest.skill.tags.is_empty() {
            md.push_str(&format!(
                "- **Tags:** {}\n",
                skill.manifest.skill.tags.join(", ")
            ));
        }
        md.push_str(&format!(
            "- **Runtime:** {:?}\n",
            skill.manifest.runtime.runtime_type
        ));
        if !skill.manifest.tools.provided.is_empty() {
            let names: Vec<&str> = skill
                .manifest
                .tools
                .provided
                .iter()
                .map(|t| t.name.as_str())
                .collect();
            md.push_str(&format!("- **Tools provided:** {}\n", names.join(", ")));
        }
        if !skill.manifest.requirements.tools.is_empty() {
            md.push_str(&format!(
                "- **Required tools:** {}\n",
                skill.manifest.requirements.tools.join(", ")
            ));
        }
        md.push('\n');
    }
    md
}

pub(crate) fn cmd_skill_remove(name: &str) {
    let home = captain_home();
    let skills_dir = home.join("skills");

    let mut registry = captain_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();
    match registry.remove(name) {
        Ok(()) => println!("Removed skill: {name}"),
        Err(e) => {
            eprintln!("Failed to remove skill: {e}");
            std::process::exit(1);
        }
    }
}

fn skill_search_source_label(skill: &captain_skills::InstalledSkill) -> &'static str {
    match skill.manifest.source.as_ref() {
        Some(captain_skills::SkillSource::Bundled) => "bundled",
        Some(captain_skills::SkillSource::OpenClaw) => "openclaw",
        Some(captain_skills::SkillSource::ClawHub { .. }) => "clawhub",
        Some(captain_skills::SkillSource::Native) | None => "local",
    }
}

fn skill_search_query_tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn skill_search_text_score(tokens: &[String], fields: &[(&str, u32)]) -> u32 {
    let mut score = 0u32;
    for token in tokens {
        for (field, weight) in fields {
            if field.to_ascii_lowercase().contains(token) {
                score += *weight;
            }
        }
    }
    score
}

fn skill_search_family(query: &str) -> Option<captain_skills::families::SkillFamily> {
    captain_skills::families::known_family(query).or_else(|| {
        let normalized = query.trim().to_ascii_lowercase();
        captain_skills::families::SKILL_FAMILIES
            .iter()
            .copied()
            .find(|family| {
                family.label.to_ascii_lowercase() == normalized
                    || family
                        .label
                        .to_ascii_lowercase()
                        .replace(" and ", "-")
                        .replace(' ', "-")
                        == normalized
            })
    })
}

pub(crate) fn cmd_skill_search(query: &str) {
    let query = query.trim();
    if query.is_empty() {
        eprintln!("Search query cannot be empty.");
        std::process::exit(2);
    }

    let home = captain_home();
    let skills_dir = home.join("skills");
    let mut registry = captain_skills::registry::SkillRegistry::new(skills_dir);
    registry.load_bundled();
    if let Err(e) = registry.load_all() {
        eprintln!("Error loading skills: {e}");
        std::process::exit(1);
    }

    let family_filter = skill_search_family(query);
    let tokens = skill_search_query_tokens(query);
    let mut results: Vec<(u32, &captain_skills::InstalledSkill, &'static str)> = Vec::new();

    for skill in registry.list().into_iter().filter(|skill| skill.enabled) {
        let family_id = captain_skills::families::infer_skill_family(skill);
        let Some(family) = captain_skills::families::known_family(family_id) else {
            continue;
        };
        if family_filter.is_some_and(|wanted| wanted.id != family.id) {
            continue;
        }

        let tags = skill.manifest.skill.tags.join(" ");
        let required_tools = skill.manifest.requirements.tools.join(" ");
        let provided_tools = skill
            .manifest
            .tools
            .provided
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let context = skill.manifest.prompt_context.as_deref().unwrap_or("");
        let mut score = if family_filter.is_some() { 100 } else { 0 };
        score += skill_search_text_score(
            &tokens,
            &[
                (skill.manifest.skill.name.as_str(), 4),
                (skill.manifest.skill.description.as_str(), 3),
                (tags.as_str(), 2),
                (family.id, 2),
                (family.label, 2),
                (required_tools.as_str(), 2),
                (provided_tools.as_str(), 2),
                (context, 1),
            ],
        );
        if score > 0 {
            results.push((score, skill, family.id));
        }
    }

    results.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.manifest.skill.name.cmp(&b.1.manifest.skill.name))
    });
    results.truncate(20);

    if results.is_empty() {
        println!("No installed or bundled skills found for \"{query}\".");
        return;
    }

    println!("Skills matching \"{query}\":\n");
    for (score, skill, family_id) in results {
        let source = skill_search_source_label(skill);
        println!(
            "  {}  [{} | {} | score {}]",
            skill.manifest.skill.name, family_id, source, score
        );
        if !skill.manifest.skill.description.is_empty() {
            println!("    {}", skill.manifest.skill.description);
        }
        if !skill.manifest.requirements.tools.is_empty() {
            println!(
                "    required tools: {}",
                skill.manifest.requirements.tools.join(", ")
            );
        }
        if skill.path != Path::new("<bundled>") {
            println!("    path: {}", skill.path.display());
        }
        println!();
    }
}

pub(crate) fn cmd_skill_create() {
    let name = prompt_input("Skill name: ");
    let description = prompt_input("Description: ");
    let runtime = prompt_input("Runtime (python/node/wasm) [python]: ");
    let runtime = if runtime.is_empty() {
        "python".to_string()
    } else {
        runtime
    };

    let home = captain_home();
    let skill_dir = home.join("skills").join(&name);
    std::fs::create_dir_all(skill_dir.join("src")).unwrap_or_else(|e| {
        eprintln!("Error creating skill directory: {e}");
        std::process::exit(1);
    });

    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "0.1.0"
description = "{description}"
author = ""
license = "MIT"
tags = []

[runtime]
type = "{runtime}"
entry = "src/main.py"

[[tools.provided]]
name = "{tool_name}"
description = "{description}"
input_schema = {{ type = "object", properties = {{ input = {{ type = "string" }} }}, required = ["input"] }}

[requirements]
tools = []
capabilities = []
"#,
        tool_name = name.replace('-', "_"),
    );

    std::fs::write(skill_dir.join("skill.toml"), &manifest).unwrap();

    let entry_content = match runtime.as_str() {
        "python" => format!(
            r#"#!/usr/bin/env python3
"""Captain skill: {name}"""
import json
import sys

def main():
    payload = json.loads(sys.stdin.read())
    tool_name = payload["tool"]
    input_data = payload["input"]

    # Replace this sample logic with the skill implementation.
    result = {{"result": f"Processed: {{input_data.get('input', '')}}"}}

    print(json.dumps(result))

if __name__ == "__main__":
    main()
"#
        ),
        _ => "// Replace this sample logic with the skill implementation.\n".to_string(),
    };

    let entry_path = if runtime == "python" {
        "src/main.py"
    } else {
        "src/index.js"
    };
    std::fs::write(skill_dir.join(entry_path), entry_content).unwrap();

    println!("\nSkill created: {}", skill_dir.display());
    println!("\nFiles:");
    println!("  skill.toml");
    println!("  {entry_path}");
    println!("\nNext steps:");
    println!("  1. Edit the entry point to implement your skill logic");
    println!("  2. Test locally: captain skill test");
    println!(
        "  3. Install: captain skill install {}",
        skill_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_search_tokenizes_case_insensitively() {
        assert_eq!(
            skill_search_query_tokens("  Browser  Automation "),
            vec!["browser", "automation"]
        );
    }

    #[test]
    fn skill_search_scores_weighted_fields() {
        let tokens = skill_search_query_tokens("web api");
        assert_eq!(
            skill_search_text_score(&tokens, &[("web browser", 4), ("api tests", 2)]),
            6
        );
    }
}
