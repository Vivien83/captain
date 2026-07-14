//! Skill family taxonomy.
//!
//! Families are discovery metadata, not filesystem layout. Generated skills can
//! keep their current directory while still being surfaced by `skill_search`.

use crate::{InstalledSkill, SkillManifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SkillFamily {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

pub const SKILL_FAMILIES: &[SkillFamily] = &[
    SkillFamily {
        id: "software-development",
        label: "Software development",
        description:
            "Planning, implementation, coding agents, TDD, debugging, and engineering workflows.",
    },
    SkillFamily {
        id: "project-management",
        label: "Project management",
        description:
            "Project planning, milestones, goals, stakeholder communication, and delivery tracking.",
    },
    SkillFamily {
        id: "review-release",
        label: "Review and release",
        description:
            "Code review, security review, CI/CD, release readiness, and publication gates.",
    },
    SkillFamily {
        id: "platform-devops",
        label: "Platform and DevOps",
        description:
            "Cloud, containers, infrastructure, deployment, monitoring, and system administration.",
    },
    SkillFamily {
        id: "data-ai",
        label: "Data and AI",
        description: "Data analysis, pipelines, ML, LLM finetuning, vector search, and databases.",
    },
    SkillFamily {
        id: "product-design",
        label: "Product and design",
        description: "Frontend, UI, writing, presentations, and product-facing artifacts.",
    },
    SkillFamily {
        id: "business-tools",
        label: "Business tools",
        description:
            "GitHub, Jira, Linear, Slack, Notion, Confluence, email, and workflow integrations.",
    },
    SkillFamily {
        id: "security-compliance",
        label: "Security and compliance",
        description:
            "Security audit, OAuth, compliance, crypto, secrets, and privacy-sensitive work.",
    },
    SkillFamily {
        id: "general-automation",
        label: "General automation",
        description:
            "Reusable automation, shell workflows, search, documents, and generated skills.",
    },
];

pub fn known_family(id: &str) -> Option<SkillFamily> {
    SKILL_FAMILIES
        .iter()
        .copied()
        .find(|family| family.id == id)
}

pub fn infer_skill_family(skill: &InstalledSkill) -> &'static str {
    infer_manifest_family(&skill.manifest)
}

pub fn infer_manifest_family(manifest: &SkillManifest) -> &'static str {
    for tag in &manifest.skill.tags {
        if let Some(family) = tag.strip_prefix("family:").and_then(known_family) {
            return family.id;
        }
    }

    let text = format!(
        "{} {} {} {}",
        manifest.skill.name,
        manifest.skill.description,
        manifest.skill.tags.join(" "),
        manifest
            .requirements
            .tools
            .iter()
            .chain(manifest.tools.provided.iter().map(|tool| &tool.name))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    )
    .to_ascii_lowercase();

    if any(
        &text,
        &[
            "review",
            "release",
            "ci",
            "cd",
            "publish",
            "pre-commit",
            "pull request",
            "pr ",
        ],
    ) {
        return "review-release";
    }
    if any(
        &text,
        &[
            "security",
            "audit",
            "compliance",
            "crypto",
            "secret",
            "oauth",
            "vulnerability",
        ],
    ) {
        return "security-compliance";
    }
    if any(
        &text,
        &[
            "project",
            "milestone",
            "stakeholder",
            "agile",
            "planning",
            "sprint",
            "goal",
        ],
    ) {
        return "project-management";
    }
    if any(
        &text,
        &[
            "docker",
            "kubernetes",
            "terraform",
            "aws",
            "gcp",
            "azure",
            "nginx",
            "ansible",
            "helm",
            "prometheus",
            "sysadmin",
            "networking",
            "deployment",
            "sentry",
        ],
    ) {
        return "platform-devops";
    }
    if any(
        &text,
        &[
            "data",
            "sql",
            "sqlite",
            "postgres",
            "mongodb",
            "redis",
            "elastic",
            "ml",
            "llm",
            "finetuning",
            "vector",
            "analyst",
            "pipeline",
        ],
    ) {
        return "data-ai";
    }
    if any(
        &text,
        &[
            "frontend",
            "css",
            "figma",
            "writing",
            "writer",
            "presentation",
            "design",
            "email",
            "technical-writer",
        ],
    ) {
        return "product-design";
    }
    if any(
        &text,
        &[
            "github",
            "jira",
            "linear",
            "slack",
            "notion",
            "confluence",
            "oauth",
        ],
    ) {
        return "business-tools";
    }
    if any(
        &text,
        &[
            "development",
            "coding",
            "code",
            "debug",
            "test",
            "tdd",
            "refactor",
            "typescript",
            "rust",
            "python",
            "react",
            "nextjs",
            "golang",
            "wasm",
            "api",
            "openapi",
            "graphql",
            "git-expert",
        ],
    ) {
        return "software-development";
    }
    "general-automation"
}

fn any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SkillMeta, SkillRuntime, SkillRuntimeConfig, SkillSource, SkillTools};

    fn manifest(name: &str, description: &str, tags: Vec<&str>) -> SkillManifest {
        SkillManifest {
            skill: SkillMeta {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                description: description.to_string(),
                author: String::new(),
                license: String::new(),
                tags: tags.into_iter().map(str::to_string).collect(),
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::PromptOnly,
                entry: String::new(),
            },
            tools: SkillTools::default(),
            requirements: Default::default(),
            prompt_context: None,
            source: Some(SkillSource::Bundled),
        }
    }

    #[test]
    fn explicit_family_tag_wins() {
        let m = manifest("custom", "whatever", vec!["family:review-release"]);
        assert_eq!(infer_manifest_family(&m), "review-release");
    }

    #[test]
    fn generated_debug_skill_maps_to_software_development() {
        let m = manifest(
            "pytest-failure-loop",
            "Debug failing tests and patch the workflow",
            vec!["generated"],
        );
        assert_eq!(infer_manifest_family(&m), "software-development");
    }
}
