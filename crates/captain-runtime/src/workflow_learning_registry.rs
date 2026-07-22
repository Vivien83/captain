//! Exact registry verification for a promoted workflow-learning artifact.

use captain_capspec::{CapabilityRegistry, CapabilityScope, CapabilityStatus};
use captain_skills::registry::SkillRegistry;

use crate::workflow_learning_promotion_types::{
    PreparedWorkflowPromotion, VerifiedWorkflowPromotion, WorkflowPromotionError,
    WorkflowPromotionPhase, WorkflowPromotionTargetKind,
};

pub fn verify_promoted_skill(
    promotion: &PreparedWorkflowPromotion,
    registry: &mut SkillRegistry,
) -> Result<VerifiedWorkflowPromotion, WorkflowPromotionError> {
    ensure_promoted(promotion, WorkflowPromotionTargetKind::Skill)?;
    registry
        .reconcile_learned_overlay(
            &promotion.manifest.target_name,
            &promotion.target_path,
            true,
        )
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    let active = registry
        .get(&promotion.manifest.target_name)
        .ok_or_else(|| {
            WorkflowPromotionError::RegistryVerification(
                "promoted skill is absent after registry reconciliation".to_string(),
            )
        })?;
    if active.path != promotion.target_path {
        return Err(WorkflowPromotionError::RegistryVerification(
            "promoted skill is not the active registry owner".to_string(),
        ));
    }
    Ok(VerifiedWorkflowPromotion::exact(&promotion.manifest))
}

pub fn verify_promoted_capspec(
    promotion: &PreparedWorkflowPromotion,
    registry: &CapabilityRegistry,
    actor: &str,
) -> Result<VerifiedWorkflowPromotion, WorkflowPromotionError> {
    ensure_promoted(promotion, WorkflowPromotionTargetKind::Capspec)?;
    if actor.trim().is_empty() {
        return Err(WorkflowPromotionError::RegistryVerification(
            "CapSpec approval actor is empty".to_string(),
        ));
    }
    registry
        .reload_global()
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    let scope = CapabilityScope::Global;
    let mut view = registry
        .capability(&scope, &promotion.manifest.target_name)
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    let expected_registry_hash =
        blake3::hash(&std::fs::read(&promotion.target_path).map_err(WorkflowPromotionError::Io)?)
            .to_hex()
            .to_string();
    if view.active_hash.as_deref() != Some(expected_registry_hash.as_str()) {
        if view.pending_hash.as_deref() != Some(expected_registry_hash.as_str()) {
            return Err(WorkflowPromotionError::RegistryVerification(
                "CapSpec registry did not load the exact promoted revision".to_string(),
            ));
        }
        view = registry
            .approve(
                &scope,
                &promotion.manifest.target_name,
                &expected_registry_hash,
                actor,
            )
            .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    }
    let registered_path = view
        .source_path
        .canonicalize()
        .map_err(WorkflowPromotionError::Io)?;
    let promoted_path = promotion
        .target_path
        .canonicalize()
        .map_err(WorkflowPromotionError::Io)?;
    if view.active_hash.as_deref() != Some(expected_registry_hash.as_str())
        || registered_path != promoted_path
    {
        return Err(WorkflowPromotionError::RegistryVerification(
            "CapSpec registry activation does not match promoted source".to_string(),
        ));
    }
    Ok(VerifiedWorkflowPromotion::exact(&promotion.manifest))
}

pub fn verify_skill_rollback(
    promotion: &PreparedWorkflowPromotion,
    registry: &mut SkillRegistry,
) -> Result<(), WorkflowPromotionError> {
    if promotion.manifest.target_kind != WorkflowPromotionTargetKind::Skill
        || !matches!(
            promotion.manifest.phase,
            WorkflowPromotionPhase::RolledBack | WorkflowPromotionPhase::Quarantined
        )
    {
        return Err(WorkflowPromotionError::InvalidPhase {
            expected: "rolled_back or quarantined skill",
            actual: promotion.manifest.phase,
        });
    }
    registry
        .reconcile_learned_overlay(
            &promotion.manifest.target_name,
            &promotion.target_path,
            false,
        )
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))
}

pub fn verify_capspec_rollback(
    promotion: &PreparedWorkflowPromotion,
    registry: &CapabilityRegistry,
    actor: &str,
) -> Result<(), WorkflowPromotionError> {
    if promotion.manifest.target_kind != WorkflowPromotionTargetKind::Capspec
        || !matches!(
            promotion.manifest.phase,
            WorkflowPromotionPhase::RolledBack | WorkflowPromotionPhase::Quarantined
        )
    {
        return Err(WorkflowPromotionError::InvalidPhase {
            expected: "rolled_back or quarantined CapSpec",
            actual: promotion.manifest.phase,
        });
    }
    if actor.trim().is_empty() {
        return Err(WorkflowPromotionError::RegistryVerification(
            "CapSpec rollback actor is empty".to_string(),
        ));
    }

    registry
        .reload_global()
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    let scope = CapabilityScope::Global;
    let mut view = registry
        .capability(&scope, &promotion.manifest.target_name)
        .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;

    if promotion.manifest.previous_sha256.is_none() {
        if promotion.target_path.exists()
            || view.status != CapabilityStatus::Disabled
            || view.active_hash.is_some()
            || view.pending_hash.is_some()
        {
            return Err(WorkflowPromotionError::RegistryVerification(
                "CapSpec rollback did not disable the removed learned source".to_string(),
            ));
        }
        return Ok(());
    }

    let previous_bytes =
        std::fs::read(&promotion.target_path).map_err(WorkflowPromotionError::Io)?;
    let expected_registry_hash = blake3::hash(&previous_bytes).to_hex().to_string();
    if view.active_hash.as_deref() != Some(expected_registry_hash.as_str()) {
        if view.pending_hash.as_deref() != Some(expected_registry_hash.as_str()) {
            return Err(WorkflowPromotionError::RegistryVerification(
                "CapSpec registry did not load the restored revision".to_string(),
            ));
        }
        view = registry
            .approve(
                &scope,
                &promotion.manifest.target_name,
                &expected_registry_hash,
                actor,
            )
            .map_err(|error| WorkflowPromotionError::RegistryVerification(error.to_string()))?;
    }
    let registered_path = view
        .source_path
        .canonicalize()
        .map_err(WorkflowPromotionError::Io)?;
    let restored_path = promotion
        .target_path
        .canonicalize()
        .map_err(WorkflowPromotionError::Io)?;
    if view.status != CapabilityStatus::Operational
        || view.active_hash.as_deref() != Some(expected_registry_hash.as_str())
        || view.pending_hash.is_some()
        || registered_path != restored_path
    {
        return Err(WorkflowPromotionError::RegistryVerification(
            "CapSpec registry activation does not match restored source".to_string(),
        ));
    }
    Ok(())
}

fn ensure_promoted(
    promotion: &PreparedWorkflowPromotion,
    expected_kind: WorkflowPromotionTargetKind,
) -> Result<(), WorkflowPromotionError> {
    if promotion.manifest.target_kind != expected_kind {
        return Err(WorkflowPromotionError::RegistryVerification(
            "registry verifier does not match promotion target kind".to_string(),
        ));
    }
    if !matches!(
        promotion.manifest.phase,
        WorkflowPromotionPhase::Promoted
            | WorkflowPromotionPhase::RegistryVerified
            | WorkflowPromotionPhase::Active
    ) {
        return Err(WorkflowPromotionError::InvalidPhase {
            expected: "promoted, registry_verified, or active",
            actual: promotion.manifest.phase,
        });
    }
    Ok(())
}
