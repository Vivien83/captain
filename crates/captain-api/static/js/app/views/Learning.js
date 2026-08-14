import { h } from '/assets/app/vendor/preact.module.js';
import { useState, useEffect, useCallback, useMemo } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

const STATE_LABELS = {
  observed: 'Observé', eligible: 'Éligible', drafting: 'Génération', validating: 'Validation',
  proposed: 'À décider', dismissed: 'Ignoré', snoozed: 'Reporté', superseded: 'Remplacé',
  approved_pending_install: 'Installation', active_canary: 'Canary', active: 'Actif',
  rejected: 'Rejeté', install_failed: 'Échec installation', rolled_back: 'Rollback effectué',
};

const KIND_LABELS = { skill: 'Skill', capspec: 'CapSpec', automation: 'Automation', refinement: 'Amélioration' };
const ACTION_LABELS = { activate: 'Activer', test: 'Tester', later: 'Reporter', ignore: 'Ignorer' };
const MUTATING_ACTIONS = new Set(['activate', 'test']);
const RUNTIME_LABELS = {
  disabled: 'Désactivé', starting: 'Démarrage', healthy: 'Opérationnel', active: 'Actif',
  recovering: 'Reprise automatique', degraded: 'Learning à vérifier', stalled: 'Worker bloqué',
};
const RECOVERY_LABELS = {
  disabled: 'désactivée', starting: 'démarrage', in_sync: 'synchronisée',
  automatic_retry_active: 'retry automatique', operator_attention: 'attention requise',
};
const JOB_ERROR_LABELS = {
  model_timeout: 'délai du modèle dépassé',
  model_completion_failed: 'appel au modèle interrompu',
  invalid_structured_output: 'réponse structurée invalide',
  invalid_draft: 'brouillon invalide',
  effect_interrupted: 'effet interrompu, rejeu automatique interdit',
  attempts_exhausted: 'nombre maximal d’essais atteint',
};
const JOB_STAGE_LABELS = {
  analyze: 'analyse', draft: 'génération', validate: 'validation',
  install: 'installation', canary: 'canary', rollback: 'rollback',
};

function relativeAge(now, at) {
  if (!Number.isFinite(now) || !Number.isFinite(at)) return 'en attente';
  const seconds = Math.max(0, Math.floor((now - at) / 1000));
  if (seconds < 60) return `il y a ${seconds}s`;
  if (seconds < 3600) return `il y a ${Math.floor(seconds / 60)}min`;
  return `il y a ${Math.floor(seconds / 3600)}h`;
}

function retryDelay(now, at) {
  if (!Number.isFinite(now) || !Number.isFinite(at)) return 'en attente';
  const seconds = Math.max(0, Math.ceil((at - now) / 1000));
  if (seconds < 60) return `dans ${seconds}s`;
  if (seconds < 3600) return `dans ${Math.ceil(seconds / 60)}min`;
  return `dans ${Math.ceil(seconds / 3600)}h`;
}

function modelLabel(model) {
  return model ? `${model.provider}:${model.model}` : 'pas encore lié';
}

function workflowName(workflow) {
  return workflow.card?.name || workflow.name || 'Workflow en construction';
}

function workflowPurpose(workflow) {
  return workflow.card?.purpose || 'Captain collecte ou valide encore les preuves de ce workflow.';
}

function actionable(workflow) {
  return workflow.projection_status === 'verified' && workflow.card?.state === 'proposed';
}

function visibleActions(workflow) {
  if (!actionable(workflow)) return [];
  return (workflow.card.available_actions || []).filter((action) => ACTION_LABELS[action]);
}

function workflowCounts(workflows, attention) {
  const counts = { total: workflows.length, decisions: 0, processing: 0, active: 0, attention: 0 };
  const attentionIds = new Set((attention || []).map((item) => item.proposal_id));
  workflows.forEach((workflow) => {
    if (attentionIds.has(workflow.proposal_id)) {
      counts.attention += 1;
      return;
    }
    if (workflow.projection_status === 'invalid' || ['rejected', 'install_failed', 'rolled_back'].includes(workflow.state)) counts.attention += 1;
    if (workflow.state === 'proposed') counts.decisions += 1;
    if (workflow.state === 'active') counts.active += 1;
    if (['observed', 'eligible', 'drafting', 'validating', 'approved_pending_install', 'active_canary'].includes(workflow.state)) counts.processing += 1;
  });
  return counts;
}

function jobErrorLabel(code) {
  return JOB_ERROR_LABELS[code] || code || 'cause non classée';
}

function memoryDisplay(metrics) {
  if (!metrics) return { className: '', label: 'mémoire en vérification', healthy: false };
  if (!metrics.learning_enabled) return { className: 'off', label: 'mémoire désactivée', healthy: false };
  if (metrics.memory_writes?.recovery === 'in_sync') return { className: 'ok', label: 'mémoire active', healthy: true };
  return { className: 'warn', label: 'mémoire en reprise', healthy: false };
}

export function Learning() {
  const [pending, setPending] = useState(null);
  const [committed, setCommitted] = useState(null);
  const [metrics, setMetrics] = useState(null);
  const [workflows, setWorkflows] = useState(null);
  const [runtimeStatus, setRuntimeStatus] = useState(null);
  const [workflowFilter, setWorkflowFilter] = useState('decisions');
  const [expandedId, setExpandedId] = useState(null);
  const [busyId, setBusyId] = useState(null);
  const [busyRecoveryId, setBusyRecoveryId] = useState(null);

  const load = useCallback(async () => {
    try {
      const [rev, com, met, learned, runtime] = await Promise.all([
        api.learningReview(), api.learningCommitted(), api.learningMetrics(), api.workflowLearning(), api.learningStatus(),
      ]);
      setPending(rev.pending || []);
      setCommitted(com.committed || []);
      setMetrics(met);
      setWorkflows(learned.workflows || []);
      setRuntimeStatus(runtime);
    } catch (e) {
      toast(`Chargement impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => {
    load();
    const timer = setInterval(load, 8000);
    return () => clearInterval(timer);
  }, [load]);

  const decideMemory = async (id, approve) => {
    setBusyId(id);
    try {
      await api.learningDecide(id, approve);
      toast(approve ? 'Mémoire approuvée' : 'Mémoire refusée');
      await load();
    } catch (e) {
      toast(`Action impossible : ${e.message}`, 'err');
    } finally {
      setBusyId(null);
    }
  };

  const decideWorkflow = async (workflow, action) => {
    if (!workflow.card || !visibleActions(workflow).includes(action)) return;
    setBusyId(workflow.proposal_id);
    try {
      await api.workflowLearningDecide(workflow.card.lookup_token, workflow.card.decision_version, action);
      toast(`${ACTION_LABELS[action]} : décision enregistrée`);
      await load();
    } catch (e) {
      toast(`Décision impossible : ${e.message}`, 'err');
      await load();
    } finally {
      setBusyId(null);
    }
  };

  const retryWorkflow = async (incident) => {
    if (!incident?.retry_available || !incident.error_code) return;
    setBusyRecoveryId(incident.proposal_id);
    try {
      const result = await api.workflowLearningRetry(incident.proposal_id, incident.error_code);
      toast(result.replayed ? 'La reprise était déjà planifiée' : 'Workflow remis en file de génération');
      await load();
    } catch (e) {
      toast(`Relance impossible : ${e.message}`, 'err');
      await load();
    } finally {
      setBusyRecoveryId(null);
    }
  };

  const incidents = runtimeStatus?.attention || [];
  const incidentIds = useMemo(() => new Set(incidents.map((item) => item.proposal_id)), [runtimeStatus]);
  const counts = useMemo(() => workflowCounts(workflows || [], incidents), [workflows, runtimeStatus]);
  const filteredWorkflows = useMemo(() => {
    if (!workflows) return [];
    if (workflowFilter === 'all') return workflows;
    if (workflowFilter === 'processing') return workflows.filter((workflow) => !incidentIds.has(workflow.proposal_id) && ['observed', 'eligible', 'drafting', 'validating', 'approved_pending_install', 'active_canary'].includes(workflow.state));
    if (workflowFilter === 'active') return workflows.filter((workflow) => workflow.state === 'active');
    if (workflowFilter === 'attention') return workflows.filter((workflow) => incidentIds.has(workflow.proposal_id) || workflow.projection_status === 'invalid' || ['rejected', 'install_failed', 'rolled_back'].includes(workflow.state));
    return workflows.filter((workflow) => workflow.state === 'proposed' || workflow.projection_status === 'invalid' || incidentIds.has(workflow.proposal_id));
  }, [workflows, workflowFilter, incidentIds]);
  const primaryIncident = incidents[0] || null;
  const memoryState = memoryDisplay(metrics);

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Learning</h1>
        <p class="page-sub">Mémoire durable et workflows réutilisables appris à partir de l'usage réel.</p>

        ${runtimeStatus === null && html`<div class="skeleton learning-runtime-skeleton"></div>`}
        ${runtimeStatus && html`
          <section class=${`learning-runtime-strip state-${runtimeStatus.state}`} data-learning-state=${runtimeStatus.state} aria-label="État opérationnel Learning">
            <div class="learning-runtime-heading">
              <span class="learning-runtime-state-dot" aria-hidden="true"></span>
              <strong>${RUNTIME_LABELS[runtimeStatus.state] || runtimeStatus.state}</strong>
              <span>mode ${runtimeStatus.mode}</span>
            </div>
            <div class="learning-runtime-cell">
              <span>Modèle lié</span>
              <strong>${modelLabel(runtimeStatus.worker?.bound_model)}</strong>
              <small>attendu ${modelLabel(runtimeStatus.expected_model)}</small>
            </div>
            <div class="learning-runtime-cell">
              <span>Worker</span>
              <strong>${runtimeStatus.worker ? `heartbeat ${relativeAge(runtimeStatus.generated_at_unix_ms, runtimeStatus.worker.heartbeat_at_unix_ms)}` : 'absent'}</strong>
              <small>${runtimeStatus.worker?.last_scan_at_unix_ms ? `scan ${relativeAge(runtimeStatus.generated_at_unix_ms, runtimeStatus.worker.last_scan_at_unix_ms)}` : 'scan en attente'}${runtimeStatus.worker?.last_error_scope ? ` · erreur ${runtimeStatus.worker.last_error_scope}` : ''}</small>
            </div>
            <div class="learning-runtime-cell">
              <span>Files</span>
              <strong>${runtimeStatus.jobs.pending} en attente · ${runtimeStatus.jobs.running} actif</strong>
              <small>${runtimeStatus.jobs.retry_wait} retry · ${runtimeStatus.jobs.uncertain} incertain · ${runtimeStatus.jobs.dead} bloqué</small>
            </div>
            <div class="learning-runtime-cell">
              <span>Reprise</span>
              <strong>${RECOVERY_LABELS[runtimeStatus.recovery] || runtimeStatus.recovery}</strong>
              <small>${runtimeStatus.jobs.next_retry_at_unix_ms
                ? `prochain retry ${retryDelay(runtimeStatus.generated_at_unix_ms, runtimeStatus.jobs.next_retry_at_unix_ms)}`
                : runtimeStatus.jobs.dead > 0
                  ? 'relance automatique arrêtée après épuisement des essais'
                  : 'aucune reprise nécessaire'}</small>
            </div>
          </section>
          ${primaryIncident && html`
            <section class="learning-runtime-alert" aria-label="Incident du moteur Learning">
              <div>
                <strong>${primaryIncident.state === 'uncertain' ? 'Rejeu automatique bloqué' : 'Génération arrêtée'}</strong>
                <span>${`${JOB_STAGE_LABELS[primaryIncident.stage] || primaryIncident.stage} · ${jobErrorLabel(primaryIncident.error_code)} · ${primaryIncident.attempt_count}/${primaryIncident.max_attempts} essais.`}</span>
                <small>${memoryState.healthy
                  ? 'La mémoire durable reste active et n’est pas affectée.'
                  : 'Cet incident Workflow Learning est distinct de l’état de la mémoire durable affiché ci-dessous.'}</small>
              </div>
              ${primaryIncident.retry_available && html`
                <button
                  class="primary"
                  disabled=${busyRecoveryId === primaryIncident.proposal_id}
                  onClick=${() => retryWorkflow(primaryIncident)}
                >${busyRecoveryId === primaryIncident.proposal_id ? 'Relance…' : 'Relancer ce workflow'}</button>
              `}
              ${!primaryIncident.retry_available && html`<small>Une vérification opérateur est requise avant toute nouvelle action.</small>`}
            </section>
          `}
        `}

        <h2 class="section-title">Workflows appris</h2>
        ${workflows === null && html`<div class="skeleton" style="height:90px;margin-bottom:18px"></div>`}
        ${workflows && html`
          <div class="metrics-row">
            <button class=${`metric-chip ${workflowFilter === 'decisions' ? 'ok' : ''}`} onClick=${() => setWorkflowFilter('decisions')}>${counts.decisions} à décider</button>
            <button class=${`metric-chip ${workflowFilter === 'all' ? 'ok' : ''}`} onClick=${() => setWorkflowFilter('all')}>${counts.total} au total</button>
            <button class=${`metric-chip ${workflowFilter === 'processing' ? 'ok' : ''}`} onClick=${() => setWorkflowFilter('processing')}>${counts.processing} en cours</button>
            <button class=${`metric-chip ${workflowFilter === 'active' ? 'ok' : ''}`} onClick=${() => setWorkflowFilter('active')}>${counts.active} actifs</button>
            ${counts.attention > 0 && html`<button class=${`metric-chip off ${workflowFilter === 'attention' ? 'selected' : ''}`} onClick=${() => setWorkflowFilter('attention')}>${counts.attention} à examiner</button>`}
          </div>
        `}
        ${workflows && filteredWorkflows.length === 0 && html`
          <div class="empty-state"><div>Aucun workflow dans ce filtre.</div></div>
        `}
        ${workflows && filteredWorkflows.length > 0 && html`
          <div class="item-list learned-workflow-list">
            ${filteredWorkflows.map((workflow) => {
              const card = workflow.card;
              const expanded = expandedId === workflow.proposal_id;
              const actions = visibleActions(workflow);
              return html`
                <div class="workflow-entry learned-workflow-entry" key=${workflow.proposal_id}>
                  <div class="item-row">
                    <div class="item-row-main">
                      <strong>${workflowName(workflow)}</strong>
                      <span class="item-row-meta">
                        ${STATE_LABELS[workflow.state] || workflow.state} · ${KIND_LABELS[workflow.kind] || 'Classification en cours'}
                        ${workflow.installation ? ` · ${workflow.installation.phase}` : ''}
                      </span>
                      <span class="item-row-meta">${workflowPurpose(workflow)}</span>
                    </div>
                    <div class="item-row-actions">
                      ${actions.map((action) => html`
                        <button
                          class=${MUTATING_ACTIONS.has(action) ? 'primary' : (action === 'ignore' ? 'danger' : '')}
                          disabled=${busyId === workflow.proposal_id}
                          onClick=${() => decideWorkflow(workflow, action)}
                        >${ACTION_LABELS[action]}</button>
                      `)}
                      <button onClick=${() => setExpandedId(expanded ? null : workflow.proposal_id)}>${expanded ? 'Masquer' : 'Détails'}</button>
                    </div>
                  </div>
                  ${expanded && html`
                    <div class="workflow-detail learned-workflow-detail">
                      ${card && html`
                        <div class="learned-workflow-facts">
                          <span><strong>Déclencheur</strong>${card.trigger}</span>
                          <span><strong>Preuves</strong>${card.evidence.occurrences} usages · ${card.evidence.distinct_sessions} sessions</span>
                          <span><strong>Validation</strong>${card.validation.length} contrôles · ${card.validated_by.provider}:${card.validated_by.model}</span>
                          <span><strong>Autorités</strong>${(card.required_authority || []).join(', ') || 'Aucune déclarée'}</span>
                        </div>
                      `}
                      ${workflow.installation && html`
                        <div class="item-row-meta">Cible : ${workflow.installation.target_locator}</div>
                      `}
                      ${(workflow.projection_error || workflow.last_error_message) && html`
                        <div class="learning-integrity-error">${workflow.projection_error || workflow.last_error_message}</div>
                      `}
                      <div class="item-row-meta">Révision ${workflow.revision_sha256 ? workflow.revision_sha256.slice(0, 12) : 'en attente'} · ${workflow.timeline.length} événements durables</div>
                    </div>
                  `}
                </div>
              `;
            })}
          </div>
        `}

        ${metrics && html`
          <div class="metrics-row learning-memory-metrics">
            <div class="metric-chip">${metrics.review_queue_pending ?? (pending || []).length} mémoires en attente</div>
            <div class="metric-chip">${(committed || []).length} retenues récentes</div>
            <div class="metric-chip">mode : ${metrics.learning_mode || 'n/a'}</div>
            <div class=${`metric-chip ${memoryState.className}`}>${memoryState.label}</div>
          </div>
        `}

        <h2 class="section-title">Mémoires à réviser</h2>
        ${pending === null && html`<div class="skeleton" style="height:70px;margin-bottom:10px"></div>`}
        ${pending && pending.length === 0 && html`<div class="empty-state"><div>Rien à réviser pour l'instant.</div></div>`}
        ${pending && pending.length > 0 && html`
          <div class="item-list">
            ${pending.map((item) => html`
              <div class="item-row" key=${item.id}>
                <div class="item-row-main">
                  <strong>${item.subject || '—'}</strong>
                  <span class="item-row-meta">${item.outcome || item.kind || ''} ${item.predicate ? `· ${item.predicate}` : ''} ${item.object ? `→ ${item.object}` : ''}</span>
                </div>
                <div class="item-row-actions">
                  <button class="primary" disabled=${busyId === item.id} onClick=${() => decideMemory(item.id, true)}>Approuver</button>
                  <button class="danger" disabled=${busyId === item.id} onClick=${() => decideMemory(item.id, false)}>Refuser</button>
                </div>
              </div>
            `)}
          </div>
        `}

        <h2 class="section-title" style="margin-top:26px">Historique mémoire</h2>
        ${committed === null && html`<div class="skeleton" style="height:70px"></div>`}
        ${committed && committed.length === 0 && html`<div class="empty-state"><div>Aucune écriture retenue pour l'instant.</div></div>`}
        ${committed && committed.length > 0 && html`
          <div class="item-list">
            ${committed.slice(0, 60).map((item) => html`
              <div class="item-row" key=${item.id}>
                <div class="item-row-main">
                  <strong>${item.subject || '—'}</strong>
                  <span class="item-row-meta">${item.source || ''} ${item.predicate ? `· ${item.predicate}` : ''} ${item.object ? `→ ${item.object}` : ''}</span>
                </div>
              </div>
            `)}
          </div>
        `}
      </div>
    </div>
  `;
}
