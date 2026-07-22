//! Skill registry — tracks installed skills and their tools.

use crate::bundled;
use crate::openclaw_compat;
use crate::verify::SkillVerifier;
use crate::{InstalledSkill, SkillError, SkillManifest, SkillSource, SkillToolDef};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Registry of installed skills.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    /// Installed skills keyed by name.
    skills: HashMap<String, InstalledSkill>,
    /// Skills directory.
    skills_dir: PathBuf,
    /// When true, no new skills can be loaded (Stable mode).
    frozen: bool,
}

impl SkillRegistry {
    /// Create a new registry rooted at the given skills directory.
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills: HashMap::new(),
            skills_dir,
            frozen: false,
        }
    }

    /// Create a cheap owned snapshot of this registry.
    ///
    /// Used to avoid holding `RwLockReadGuard` across `.await` points
    /// (the guard is `!Send`).
    pub fn snapshot(&self) -> SkillRegistry {
        SkillRegistry {
            skills: self.skills.clone(),
            skills_dir: self.skills_dir.clone(),
            frozen: self.frozen,
        }
    }

    /// Freeze the registry, preventing any new skills from being loaded.
    /// Used in Stable mode after initial boot.
    pub fn freeze(&mut self) {
        self.frozen = true;
        info!("Skill registry frozen — no new skills will be loaded");
    }

    /// Check if the registry is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Load all bundled skills (compile-time embedded SKILL.md files).
    ///
    /// Called before `load_all()` so that user-installed skills with the same name
    /// can override bundled ones. Runs prompt injection scan even on bundled skills
    /// as a defense-in-depth measure.
    pub fn load_bundled(&mut self) -> usize {
        let bundled = bundled::bundled_skills();
        let mut count = 0;

        for (name, content) in &bundled {
            match bundled::parse_bundled(name, content) {
                Ok(manifest) => {
                    // Defense in depth: scan even bundled skill prompt content
                    if let Some(ref ctx) = manifest.prompt_context {
                        let warnings = SkillVerifier::scan_prompt_content(ctx);
                        let has_critical = warnings.iter().any(|w| {
                            matches!(w.severity, crate::verify::WarningSeverity::Critical)
                        });
                        if has_critical {
                            warn!(
                                skill = %manifest.skill.name,
                                "BLOCKED bundled skill: critical prompt injection patterns"
                            );
                            continue;
                        }
                    }

                    self.skills.insert(
                        manifest.skill.name.clone(),
                        InstalledSkill {
                            manifest,
                            path: PathBuf::from("<bundled>"),
                            enabled: true,
                        },
                    );
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to parse bundled skill '{name}': {e}");
                }
            }
        }

        if count > 0 {
            info!("Loaded {count} bundled skill(s)");
        }
        count
    }

    /// Load all installed skills from the skills directory.
    pub fn load_all(&mut self) -> Result<usize, SkillError> {
        if !self.skills_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let mut paths = std::fs::read_dir(&self.skills_dir)?
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| {
            let left_learned = left.file_name().and_then(|name| name.to_str()) == Some("learned");
            let right_learned = right.file_name().and_then(|name| name.to_str()) == Some("learned");
            left_learned
                .cmp(&right_learned)
                .then_with(|| left.cmp(right))
        });

        for path in paths {
            if path.is_file() {
                if is_markdown_skill_file(&path) {
                    match self.load_markdown_skill_file(&path) {
                        Ok(_) => count += 1,
                        Err(e) => warn!("Failed to load Markdown skill at {}: {e}", path.display()),
                    }
                }
                continue;
            }
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                // Auto-detect SKILL.md and convert to skill.toml + prompt_context.md
                if openclaw_compat::detect_skillmd(&path) {
                    match openclaw_compat::convert_skillmd(&path) {
                        Ok(converted) => {
                            // SECURITY: Scan prompt content for injection attacks
                            // before accepting the skill. 341 malicious skills were
                            // found on ClawHub — block critical threats at load time.
                            let warnings =
                                SkillVerifier::scan_prompt_content(&converted.prompt_context);
                            let has_critical = warnings.iter().any(|w| {
                                matches!(w.severity, crate::verify::WarningSeverity::Critical)
                            });
                            if has_critical {
                                warn!(
                                    skill = %converted.manifest.skill.name,
                                    "BLOCKED: SKILL.md contains critical prompt injection patterns"
                                );
                                for w in &warnings {
                                    warn!("  [{:?}] {}", w.severity, w.message);
                                }
                                continue;
                            }
                            if !warnings.is_empty() {
                                for w in &warnings {
                                    warn!(
                                        skill = %converted.manifest.skill.name,
                                        "[{:?}] {}",
                                        w.severity,
                                        w.message
                                    );
                                }
                            }

                            info!(
                                skill = %converted.manifest.skill.name,
                                "Auto-converting SKILL.md to Captain format"
                            );
                            if let Err(e) =
                                openclaw_compat::write_captain_manifest(&path, &converted.manifest)
                            {
                                warn!("Failed to write skill.toml for {}: {e}", path.display());
                                continue;
                            }
                            if let Err(e) = openclaw_compat::write_prompt_context(
                                &path,
                                &converted.prompt_context,
                            ) {
                                warn!(
                                    "Failed to write prompt_context.md for {}: {e}",
                                    path.display()
                                );
                            }
                            // Fall through to load the newly written skill.toml
                        }
                        Err(e) => {
                            warn!("Failed to convert SKILL.md at {}: {e}", path.display());
                            continue;
                        }
                    }
                } else if let Ok(loaded) = self.load_markdown_skills_in_dir(&path) {
                    if loaded > 0 {
                        count += loaded;
                    }
                    continue;
                } else {
                    continue;
                }
            }

            match self.load_skill(&path) {
                Ok(_) => count += 1,
                Err(e) => {
                    warn!("Failed to load skill at {}: {e}", path.display());
                }
            }
        }

        info!("Loaded {count} skills from {}", self.skills_dir.display());
        Ok(count)
    }

    fn load_markdown_skills_in_dir(&mut self, dir: &Path) -> Result<usize, SkillError> {
        let mut paths = std::fs::read_dir(dir)?
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();
        let mut count = 0;
        for path in paths {
            if !is_markdown_skill_file(&path) {
                continue;
            }
            match self.load_markdown_skill_file(&path) {
                Ok(_) => count += 1,
                Err(e) => warn!("Failed to load Markdown skill at {}: {e}", path.display()),
            }
        }
        Ok(count)
    }

    fn load_markdown_skill_file(&mut self, path: &Path) -> Result<String, SkillError> {
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }
        let content = std::fs::read_to_string(path)?;
        let name_hint = path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("generated-skill");
        let mut converted = openclaw_compat::convert_skillmd_str(name_hint, &content)?;
        converted.manifest.source = Some(SkillSource::Native);
        converted.manifest.skill.tags.retain(|tag| tag != "bundled");
        if !converted
            .manifest
            .skill
            .tags
            .iter()
            .any(|tag| tag == "generated")
            && path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                == Some("generated")
        {
            converted.manifest.skill.tags.push("generated".to_string());
        }
        if path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("learned")
        {
            for tag in ["generated", "learned"] {
                if !converted
                    .manifest
                    .skill
                    .tags
                    .iter()
                    .any(|existing| existing == tag)
                {
                    converted.manifest.skill.tags.push(tag.to_string());
                }
            }
        }

        if let Some(ref ctx) = converted.manifest.prompt_context {
            let warnings = SkillVerifier::scan_prompt_content(ctx);
            let has_critical = warnings
                .iter()
                .any(|w| matches!(w.severity, crate::verify::WarningSeverity::Critical));
            if has_critical {
                return Err(SkillError::SecurityBlocked(format!(
                    "critical prompt injection pattern in {}",
                    path.display()
                )));
            }
            for warning in warnings {
                warn!(
                    skill = %converted.manifest.skill.name,
                    "[{:?}] {}",
                    warning.severity,
                    warning.message
                );
            }
        }

        let name = converted.manifest.skill.name.clone();
        self.skills.insert(
            name.clone(),
            InstalledSkill {
                manifest: converted.manifest,
                path: path.to_path_buf(),
                enabled: true,
            },
        );
        info!("Loaded Markdown skill: {name}");
        Ok(name)
    }

    /// Load a single skill from a directory.
    pub fn load_skill(&mut self, skill_dir: &Path) -> Result<String, SkillError> {
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }
        let manifest_path = skill_dir.join("skill.toml");
        let toml_str = std::fs::read_to_string(&manifest_path)?;
        let manifest: SkillManifest = toml::from_str(&toml_str)?;

        let name = manifest.skill.name.clone();

        self.skills.insert(
            name.clone(),
            InstalledSkill {
                manifest,
                path: skill_dir.to_path_buf(),
                enabled: true,
            },
        );

        info!("Loaded skill: {name}");
        Ok(name)
    }

    /// Get an installed skill by name.
    pub fn get(&self, name: &str) -> Option<&InstalledSkill> {
        self.skills.get(name)
    }

    /// Rebuild the registry and prove that one controlled learning overlay is
    /// either the exact active owner or has been fully removed. Stable mode is
    /// preserved, but cannot turn a successfully approved install into a
    /// restart-only feature.
    pub fn reconcile_learned_overlay(
        &mut self,
        name: &str,
        overlay_path: &Path,
        should_be_active: bool,
    ) -> Result<(), SkillError> {
        let learned_root = self.skills_dir.join("learned");
        if overlay_path.parent() != Some(learned_root.as_path())
            || overlay_path.file_stem().and_then(|value| value.to_str()) != Some(name)
            || overlay_path.extension().and_then(|value| value.to_str()) != Some("md")
        {
            return Err(SkillError::SecurityBlocked(
                "learned overlay is not a direct <name>.md child of skills/learned".to_string(),
            ));
        }
        ensure_real_directory_if_present(&learned_root)?;
        match std::fs::symlink_metadata(overlay_path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(SkillError::SecurityBlocked(
                    "learned overlay must be a regular file".to_string(),
                ))
            }
            Ok(_) if !should_be_active => {
                return Err(SkillError::SecurityBlocked(
                    "rolled-back learned overlay is still present".to_string(),
                ))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !should_be_active => {}
            Err(error) => return Err(error.into()),
        }

        let was_frozen = self.frozen;
        let mut fresh = SkillRegistry::new(self.skills_dir.clone());
        fresh.load_bundled();
        fresh.load_all()?;
        if should_be_active {
            let active = fresh.get(name).ok_or_else(|| {
                SkillError::InvalidManifest(format!(
                    "learned overlay {name} was not loaded by the registry"
                ))
            })?;
            if active.path != overlay_path {
                return Err(SkillError::SecurityBlocked(format!(
                    "learned overlay {name} did not become the active owner"
                )));
            }
        } else if fresh
            .get(name)
            .is_some_and(|active| active.path == overlay_path)
        {
            return Err(SkillError::SecurityBlocked(format!(
                "rolled-back learned overlay {name} remains active"
            )));
        }
        if was_frozen {
            fresh.freeze();
        }
        *self = fresh;
        Ok(())
    }

    /// List all installed skills.
    pub fn list(&self) -> Vec<&InstalledSkill> {
        let mut skills: Vec<&InstalledSkill> = self.skills.values().collect();
        skills.sort_by(|a, b| a.manifest.skill.name.cmp(&b.manifest.skill.name));
        skills
    }

    /// Remove a skill by name.
    pub fn remove(&mut self, name: &str) -> Result<(), SkillError> {
        let skill = self
            .skills
            .remove(name)
            .ok_or_else(|| SkillError::NotFound(name.to_string()))?;

        // Remove the skill directory
        if skill.path.exists() {
            std::fs::remove_dir_all(&skill.path)?;
        }

        info!("Removed skill: {name}");
        Ok(())
    }

    /// Get all tool definitions from all enabled skills.
    pub fn all_tool_definitions(&self) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled)
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Get tool definitions only from the named skills.
    pub fn tool_definitions_for_skills(&self, names: &[String]) -> Vec<SkillToolDef> {
        self.skills
            .values()
            .filter(|s| s.enabled && names.contains(&s.manifest.skill.name))
            .flat_map(|s| s.manifest.tools.provided.iter().cloned())
            .collect()
    }

    /// Return all installed skill names.
    pub fn skill_names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    /// Find which skill provides a given tool name.
    pub fn find_tool_provider(&self, tool_name: &str) -> Option<&InstalledSkill> {
        self.skills.values().find(|s| {
            s.enabled
                && s.manifest
                    .tools
                    .provided
                    .iter()
                    .any(|t| t.name == tool_name)
        })
    }

    /// Count installed skills.
    pub fn count(&self) -> usize {
        self.skills.len()
    }

    /// Load workspace-scoped skills that override global/bundled skills.
    ///
    /// Scans subdirectories of `workspace_skills_dir` using the same loading
    /// logic as `load_all()`: auto-converts SKILL.md, runs prompt injection
    /// scan, blocks critical threats. Skills loaded here override global ones
    /// with the same name (insert semantics).
    pub fn load_workspace_skills(
        &mut self,
        workspace_skills_dir: &Path,
    ) -> Result<usize, SkillError> {
        if !workspace_skills_dir.exists() {
            return Ok(0);
        }
        if self.frozen {
            return Err(SkillError::NotFound(
                "Skill registry is frozen (Stable mode)".to_string(),
            ));
        }

        let mut count = 0;
        let entries = std::fs::read_dir(workspace_skills_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("skill.toml");
            if !manifest_path.exists() {
                // Auto-detect SKILL.md and convert
                if openclaw_compat::detect_skillmd(&path) {
                    match openclaw_compat::convert_skillmd(&path) {
                        Ok(converted) => {
                            let warnings =
                                SkillVerifier::scan_prompt_content(&converted.prompt_context);
                            let has_critical = warnings.iter().any(|w| {
                                matches!(w.severity, crate::verify::WarningSeverity::Critical)
                            });
                            if has_critical {
                                warn!(
                                    skill = %converted.manifest.skill.name,
                                    "BLOCKED workspace skill: critical prompt injection patterns"
                                );
                                continue;
                            }

                            if let Err(e) =
                                openclaw_compat::write_captain_manifest(&path, &converted.manifest)
                            {
                                warn!("Failed to write skill.toml for {}: {e}", path.display());
                                continue;
                            }
                            if let Err(e) = openclaw_compat::write_prompt_context(
                                &path,
                                &converted.prompt_context,
                            ) {
                                warn!(
                                    "Failed to write prompt_context.md for {}: {e}",
                                    path.display()
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to convert workspace SKILL.md at {}: {e}",
                                path.display()
                            );
                            continue;
                        }
                    }
                } else {
                    continue;
                }
            }

            match self.load_skill(&path) {
                Ok(name) => {
                    info!("Loaded workspace skill: {name}");
                    count += 1;
                }
                Err(e) => {
                    warn!("Failed to load workspace skill at {}: {e}", path.display());
                }
            }
        }

        if count > 0 {
            info!(
                "Loaded {count} workspace skill(s) from {}",
                workspace_skills_dir.display()
            );
        }
        Ok(count)
    }
}

fn ensure_real_directory_if_present(path: &Path) -> Result<(), SkillError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            SkillError::SecurityBlocked(format!("{} is not a real directory", path.display())),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn is_markdown_skill_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| !name.eq_ignore_ascii_case("README.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_skill(dir: &Path, name: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.toml"),
            format!(
                r#"
[skill]
name = "{name}"
version = "0.1.0"
description = "Test skill"

[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_load_all() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "skill-a");
        create_test_skill(dir.path(), "skill-b");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 2);
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_get_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "my-skill");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let skill = registry.get("my-skill");
        assert!(skill.is_some());
        assert_eq!(skill.unwrap().manifest.skill.name, "my-skill");
    }

    #[test]
    fn test_tool_definitions() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "alpha");
        create_test_skill(dir.path(), "beta");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        let tools = registry.all_tool_definitions();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_find_tool_provider() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "finder");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();

        assert!(registry.find_tool_provider("finder_tool").is_some());
        assert!(registry.find_tool_provider("nonexistent").is_none());
    }

    #[test]
    fn test_remove_skill() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "removable");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 1);

        registry.remove("removable").unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_empty_dir() {
        let dir = TempDir::new().unwrap();
        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        assert_eq!(registry.load_all().unwrap(), 0);
    }

    #[test]
    fn test_frozen_blocks_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "blocked");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        registry.freeze();
        assert!(registry.is_frozen());

        // Trying to load a skill should fail
        let result = registry.load_skill(&dir.path().join("blocked"));
        assert!(result.is_err());
    }

    #[test]
    fn test_frozen_after_initial_load() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "initial");
        create_test_skill(dir.path(), "later");

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        // Initial load works
        registry.load_all().unwrap();
        assert_eq!(registry.count(), 2);

        // Freeze
        registry.freeze();

        // Dynamic load blocked
        create_test_skill(dir.path(), "new-skill");
        let result = registry.load_skill(&dir.path().join("new-skill"));
        assert!(result.is_err());
        // Still has the original skills
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_registry_auto_convert_skillmd() {
        let dir = TempDir::new().unwrap();

        // Create a SKILL.md-only skill (no skill.toml)
        let skill_dir = dir.path().join("writing-coach");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: writing-coach\ndescription: Helps improve writing\n---\n# Writing Coach\n\nHelp users write better.",
        ).unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 1, "Should auto-convert and load the SKILL.md skill");

        let skill = registry.get("writing-coach");
        assert!(skill.is_some());
        let manifest = &skill.unwrap().manifest;
        assert_eq!(
            manifest.runtime.runtime_type,
            crate::SkillRuntime::PromptOnly
        );
        assert!(manifest.prompt_context.is_some());

        // Verify that skill.toml was written
        assert!(skill_dir.join("skill.toml").exists());
    }

    #[test]
    fn test_registry_loads_generated_markdown_skills() {
        let dir = TempDir::new().unwrap();
        let generated = dir.path().join("generated");
        std::fs::create_dir_all(&generated).unwrap();
        std::fs::write(
            generated.join("smoke-check.md"),
            r#"---
name: smoke-check
description: Runs project smoke checks
family: software-development
tags:
  - generated
---
# Smoke check

Run the project's smoke-check workflow.
"#,
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        let count = registry.load_all().unwrap();
        assert_eq!(count, 1);
        let skill = registry.get("smoke-check").expect("generated skill loaded");
        assert_eq!(skill.manifest.source, Some(crate::SkillSource::Native));
        assert!(skill
            .manifest
            .skill
            .tags
            .contains(&"family:software-development".to_string()));
    }

    #[test]
    fn learned_markdown_overlay_deterministically_wins_by_name() {
        let dir = TempDir::new().unwrap();
        create_test_skill(dir.path(), "shared-workflow");
        let learned = dir.path().join("learned");
        std::fs::create_dir_all(&learned).unwrap();
        let overlay = learned.join("shared-workflow.md");
        std::fs::write(
            &overlay,
            r#"---
name: shared-workflow
description: Learned replacement
---
# Learned replacement

Use the verified learned procedure.
"#,
        )
        .unwrap();

        let mut registry = SkillRegistry::new(dir.path().to_path_buf());
        assert_eq!(registry.load_all().unwrap(), 2);
        let active = registry.get("shared-workflow").unwrap();
        assert_eq!(active.path, overlay);
        assert_eq!(active.manifest.skill.description, "Learned replacement");
    }
}
