import { h } from 'preact';
import { useCallback, useEffect, useState } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);
const STARTER_SOURCE = `format = 1
name = "project-summary"
description = "Read the project README and manifest in parallel."
version = "1.0.0"
output = { readme = "{{steps.readme.output}}", manifest = "{{steps.manifest.output}}" }

[inputs.root]
type = "string"
description = "Project root"

[permissions]
tools = ["file_read"]
read_paths = ["{{input.root}}/**"]

[policy]
timeout_secs = 60
max_parallel = 2

[[steps]]
id = "readme"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/README.md" }

[[steps]]
id = "manifest"
tool = "file_read"
needs = []
with = { path = "{{input.root}}/Cargo.toml" }
`;

export function NativeCapabilitiesTab() {
  const [scope, setScope] = useState('effective');
  const [workspace, setWorkspace] = useState('');
  const [query, setQuery] = useState('');
  const [data, setData] = useState(null);
  const [runs, setRuns] = useState(null);
  const [selected, setSelected] = useState(null);
  const [showForge, setShowForge] = useState(false);

  const load = useCallback(async () => {
    if (scope === 'project' && !workspace.trim()) {
      setData({ count: 0, capabilities: [] });
      return;
    }
    try {
      const params = { scope, workspace: workspace.trim() || undefined };
      const [capabilities, recentRuns] = await Promise.all([
        api.nativeCapabilities(params),
        api.nativeCapabilityRuns(20),
      ]);
      setData(capabilities);
      setRuns(recentRuns);
    } catch (error) {
      toast(`Chargement impossible : ${error.message}`, 'err');
    }
  }, [scope, workspace]);

  useEffect(() => { load(); }, [load]);

  const capabilities = sortCapabilities((data && data.capabilities) || []);
  const needle = query.trim().toLocaleLowerCase('fr');
  const filtered = needle
    ? capabilities.filter((item) => `${item.name} ${item.description || ''} ${item.status}`
      .toLocaleLowerCase('fr').includes(needle))
    : capabilities;
  const pending = capabilities.filter((item) => item.human_action_required).length;
  const operational = capabilities.filter((item) => item.ready).length;

  return html`
    <div class="native-capabilities">
      <div class="capability-toolbar native-capability-toolbar">
        <div class="metrics-row">
          <div class="metric-chip">${data ? data.count : '…'} native(s)</div>
          <div class="metric-chip ${operational ? 'ok' : ''}">${operational} prête(s)</div>
          <div class="metric-chip ${pending ? 'off' : ''}">${pending} décision(s)</div>
        </div>
        <div class="native-toolbar-actions">
          <button class="ghost" onClick=${load}>Actualiser</button>
          <button onClick=${() => setShowForge((visible) => !visible)}>
            ${showForge ? 'Fermer' : 'Installer'}
          </button>
        </div>
      </div>

      <div class="native-scope-bar">
        <select value=${scope} onChange=${(event) => setScope(event.target.value)} aria-label="Portée">
          <option value="effective">Effectif</option>
          <option value="global">Global</option>
          <option value="project">Projet</option>
          <option value="all">Global + projet</option>
        </select>
        <input type="text" value=${workspace} placeholder="Workspace projet"
          onInput=${(event) => setWorkspace(event.target.value)} />
        <input type="text" value=${query} placeholder="Filtrer les capacités"
          onInput=${(event) => setQuery(event.target.value)} />
      </div>

      ${showForge && html`<${ForgePanel} workspace=${workspace} onInstalled=${async () => {
        setShowForge(false);
        await load();
      }} />`}

      ${data === null && html`<div class="skeleton" style="height:90px"></div>`}
      ${data && scope === 'project' && !workspace.trim() && html`
        <div class="status-empty">Renseignez le workspace du projet.</div>
      `}
      ${data && filtered.length === 0 && !(scope === 'project' && !workspace.trim()) && html`
        <div class="empty-state"><div class="glyph">◇</div><div>Aucune capacité native dans cette portée.</div></div>
      `}
      ${filtered.length > 0 && html`
        <div class="item-list native-capability-list">
          ${filtered.map((item) => html`
            <${CapabilityRow}
              key=${`${item.scope}:${item.name}:${item.selected_hash || ''}`}
              item=${item}
              workspace=${workspace}
              open=${selected && selected.name === item.name && selected.scope === item.scope}
              onToggle=${() => setSelected((current) => current && current.name === item.name
                && current.scope === item.scope ? null : item)}
              onChanged=${load}
            />
          `)}
        </div>
      `}

      <${RunsPanel} data=${runs} onChanged=${load} />
    </div>
  `;
}

function ForgePanel({ workspace, onInstalled }) {
  const [source, setSource] = useState(STARTER_SOURCE);
  const [name, setName] = useState('');
  const [scope, setScope] = useState('global');
  const [validation, setValidation] = useState(null);
  const [busy, setBusy] = useState(false);

  const validate = async () => {
    setBusy(true);
    try {
      const result = await api.validateNativeCapability({
        source,
        name: name.trim() || null,
      });
      setValidation(result);
      toast(`CapSpec ${shortHash(result.source_hash)} valide.`, 'ok');
    } catch (error) {
      setValidation(null);
      toast(`CapSpec invalide : ${error.message}`, 'err');
    } finally { setBusy(false); }
  };

  const install = async () => {
    if (scope === 'project' && !workspace.trim()) {
      toast('Le workspace projet est requis.', 'err');
      return;
    }
    setBusy(true);
    try {
      const result = await api.installNativeCapability({
        source,
        name: name.trim() || null,
        scope,
        workspace: scope === 'project' ? workspace.trim() : null,
      });
      toast(result.human_action_required
        ? `Proposition ${result.name} enregistrée : approbation requise.`
        : `Capacité ${result.name} prête.`, 'ok');
      await onInstalled();
    } catch (error) {
      toast(`Installation refusée : ${error.message}`, 'err');
    } finally { setBusy(false); }
  };

  return html`
    <section class="native-forge" aria-label="Installer une capacité native">
      <div class="native-forge-head">
        <h2>Fichier .captain</h2>
        ${validation && html`
          <span class="status-pill status-done">valide · ${shortHash(validation.source_hash)}</span>
        `}
      </div>
      <div class="native-forge-controls">
        <input type="text" value=${name} placeholder="Nom attendu (optionnel)"
          onInput=${(event) => { setName(event.target.value); setValidation(null); }} />
        <select value=${scope} onChange=${(event) => setScope(event.target.value)}>
          <option value="global">Global</option>
          <option value="project">Projet courant</option>
        </select>
      </div>
      <textarea class="native-source-editor" spellcheck="false" value=${source}
        onInput=${(event) => { setSource(event.target.value); setValidation(null); }} />
      <div class="form-actions">
        <button class="ghost" disabled=${busy} onClick=${validate}>Valider</button>
        <button disabled=${busy} onClick=${install}>Installer</button>
      </div>
    </section>
  `;
}

function CapabilityRow({ item, workspace, open, onToggle, onChanged }) {
  return html`
    <div class="native-capability-entry">
      <div class="item-row">
        <button class="native-row-toggle" onClick=${onToggle} aria-expanded=${open}>
          <span class="native-row-title">
            <strong>${item.name}</strong>
            <span class="item-row-meta">${item.description || item.tool_name}</span>
            <span class="item-row-meta">
              ${item.version ? `v${item.version} · ` : ''}${item.scope} · ${shortHash(item.selected_hash)}
            </span>
          </span>
          <span class="status-pill ${statusTone(item.status)}">${statusLabel(item.status)}</span>
        </button>
        ${item.human_action_required && html`
          <${DecisionButtons} item=${item} workspace=${workspace} onChanged=${onChanged} />
        `}
      </div>
      ${open && html`<${CapabilityDetail} item=${item} workspace=${workspace} onChanged=${onChanged} />`}
    </div>
  `;
}

function DecisionButtons({ item, workspace, onChanged }) {
  const [busy, setBusy] = useState(false);
  const decide = async (decision) => {
    setBusy(true);
    try {
      await api.decideNativeCapability(item.name, {
        decision,
        expected_hash: item.pending_hash,
        scope: item.scope,
        workspace: item.scope === 'project' ? workspace.trim() : null,
      });
      toast(decision === 'approve' ? `${item.name} approuvée.` : `${item.name} refusée.`, 'ok');
      await onChanged();
    } catch (error) {
      toast(`Décision impossible : ${error.message}`, 'err');
    } finally { setBusy(false); }
  };
  return html`
    <div class="item-row-actions">
      <button class="ghost danger" disabled=${busy} onClick=${() => decide('reject')}>Refuser</button>
      <button disabled=${busy} onClick=${() => decide('approve')}>Approuver le hash</button>
    </div>
  `;
}

function CapabilityDetail({ item, workspace, onChanged }) {
  const [detail, setDetail] = useState(null);
  const [sourceVisible, setSourceVisible] = useState(false);
  const [busy, setBusy] = useState(false);
  const [disableArmed, setDisableArmed] = useState(false);

  const load = useCallback(async (includeSource = false) => {
    try {
      const result = await api.nativeCapability(item.name, {
        scope: item.scope,
        workspace: item.scope === 'project' ? workspace.trim() : undefined,
        include_source: includeSource,
      });
      setDetail(result);
      setSourceVisible(includeSource);
    } catch (error) {
      toast(`Inspection impossible : ${error.message}`, 'err');
    }
  }, [item.name, item.scope, workspace]);

  useEffect(() => { load(false); }, [load]);

  const rollback = async (hash) => {
    setBusy(true);
    try {
      await api.rollbackNativeCapability(item.name, {
        target_hash: hash,
        scope: item.scope,
        workspace: item.scope === 'project' ? workspace.trim() : null,
      });
      toast(`${item.name} restaurée sur ${shortHash(hash)}.`, 'ok');
      await load(false);
      await onChanged();
    } catch (error) {
      toast(`Rollback impossible : ${error.message}`, 'err');
    } finally { setBusy(false); }
  };

  const disable = async () => {
    if (!disableArmed) {
      setDisableArmed(true);
      return;
    }
    setBusy(true);
    try {
      await api.disableNativeCapability(item.name, {
        scope: item.scope,
        workspace: item.scope === 'project' ? workspace.trim() : undefined,
      });
      toast(`${item.name} désactivée ; historique conservé.`, 'ok');
      await onChanged();
    } catch (error) {
      toast(`Désactivation impossible : ${error.message}`, 'err');
    } finally { setBusy(false); setDisableArmed(false); }
  };

  if (!detail) return html`<div class="detail-panel"><div class="skeleton" style="height:70px"></div></div>`;
  return html`
    <div class="native-capability-detail">
      <div class="native-detail-grid">
        <div><span>Tool</span><strong>${detail.tool_name}</strong></div>
        <div><span>Hash actif</span><strong>${shortHash(detail.active_hash)}</strong></div>
        <div><span>Empreinte permissions</span><strong>${shortHash(detail.permission_fingerprint)}</strong></div>
        <div><span>Étapes</span><strong>${(detail.steps || []).length}</strong></div>
      </div>
      <div class="native-permissions">
        ${(detail.permissions && detail.permissions.tools || []).map((tool) => html`
          <span class="status-pill status-review" key=${tool}>${tool}</span>
        `)}
      </div>
      ${detail.last_error && html`<div class="status-issue tone-err"><div><strong>Erreur source</strong><span>${detail.last_error}</span></div></div>`}
      <div class="native-detail-actions">
        <button class="ghost" disabled=${busy} onClick=${() => load(!sourceVisible)}>
          ${sourceVisible ? 'Masquer la source' : 'Voir la source'}
        </button>
        <button class="ghost danger" disabled=${busy} onClick=${disable}>
          ${disableArmed ? 'Confirmer la désactivation' : 'Désactiver'}
        </button>
      </div>
      ${sourceVisible && html`<pre class="code-block">${detail.source || ''}</pre>`}
      <div class="native-revisions">
        <h3>Révisions</h3>
        ${(detail.revisions || []).map((revision) => html`
          <div class="native-revision" key=${revision.source_hash}>
            <div>
              <strong>${revision.version || 'sans version'} · ${shortHash(revision.source_hash)}</strong>
              <span>${revision.approved_by ? `approuvée par ${revision.approved_by}`
                : revision.rejected_by ? `refusée par ${revision.rejected_by}` : 'non décidée'}</span>
            </div>
            ${revision.source_hash !== detail.active_hash && html`
              <button class="ghost" disabled=${busy} onClick=${() => rollback(revision.source_hash)}>Restaurer</button>
            `}
          </div>
        `)}
      </div>
    </div>
  `;
}

function RunsPanel({ data, onChanged }) {
  const runs = (data && data.runs) || [];
  return html`
    <section class="native-runs">
      <h2>Runs récents</h2>
      ${data === null && html`<div class="skeleton" style="height:54px"></div>`}
      ${data && runs.length === 0 && html`<div class="status-empty">Aucun run natif.</div>`}
      ${runs.map((run) => html`<${RunRow} key=${run.run_id} run=${run} onChanged=${onChanged} />`)}
    </section>
  `;
}

function RunRow({ run, onChanged }) {
  const [open, setOpen] = useState(false);
  const uncertain = (run.nodes || []).find((node) => node.status === 'uncertain');
  return html`
    <div class="native-run-entry">
      <div class="native-run">
        <div>
          <strong>${run.capability_name}</strong>
          <span>${shortHash(run.run_id)} · ${shortHash(run.source_hash)} · ${run.origin || 'runtime'}</span>
        </div>
        <div class="native-run-actions">
          <span class="status-pill ${runTone(run.status)}">${String(run.status).replaceAll('_', ' ')}</span>
          ${uncertain && html`
            <button class="ghost" onClick=${() => setOpen((value) => !value)} aria-expanded=${open}>
              ${open ? 'Fermer' : 'Décider'}
            </button>
          `}
        </div>
      </div>
      ${open && uncertain && html`
        <${RunDecisionPanel} run=${run} node=${uncertain} onChanged=${async () => {
          setOpen(false);
          await onChanged();
        }} />
      `}
    </div>
  `;
}

function RunDecisionPanel({ run, node, onChanged }) {
  const [output, setOutput] = useState('null');
  const [reason, setReason] = useState('');
  const [busy, setBusy] = useState(false);

  const decide = async (decision) => {
    let parsedOutput;
    if (decision === 'confirm_succeeded') {
      try { parsedOutput = JSON.parse(output); } catch {
        toast('La sortie confirmée doit être du JSON valide.', 'err');
        return;
      }
    }
    if (decision === 'mark_failed' && !reason.trim()) {
      toast('Le motif d’échec est requis.', 'err');
      return;
    }
    setBusy(true);
    try {
      const body = {
        node_id: node.step_id,
        expected_tool_use_id: node.tool_use_id,
        expected_attempt: node.attempts,
        decision,
      };
      if (decision === 'confirm_succeeded') body.output = parsedOutput;
      if (decision === 'mark_failed') body.reason = reason.trim();
      const result = await api.resolveNativeCapabilityRun(run.run_id, body);
      toast(result.resume_scheduled
        ? `Décision acceptée pour ${node.step_id} ; reprise planifiée.`
        : `Run ${shortHash(run.run_id)} marqué en échec.`, 'ok');
      await onChanged();
    } catch (error) {
      toast(`Décision refusée : ${error.message}`, 'err');
      await onChanged();
    } finally { setBusy(false); }
  };

  return html`
    <div class="native-run-decision">
      <div class="native-detail-grid">
        <div><span>Étape</span><strong>${node.step_id}</strong></div>
        <div><span>Tool</span><strong>${node.tool_name}</strong></div>
        <div><span>Tentative</span><strong>${node.attempts}</strong></div>
        <div><span>Tool use</span><strong>${shortHash(node.tool_use_id)}</strong></div>
      </div>
      <div class="native-run-decision-fields">
        <label>Sortie observée (JSON)
          <textarea spellcheck="false" value=${output}
            onInput=${(event) => setOutput(event.target.value)} />
        </label>
        <label>Motif d’échec
          <input type="text" value=${reason}
            onInput=${(event) => setReason(event.target.value)} />
        </label>
      </div>
      <div class="native-detail-actions">
        <button class="ghost danger" disabled=${busy} onClick=${() => decide('mark_failed')}>Marquer échoué</button>
        <button class="ghost" disabled=${busy} onClick=${() => decide('retry')}>Réessayer</button>
        <button disabled=${busy} onClick=${() => decide('confirm_succeeded')}>Confirmer réussi</button>
      </div>
    </div>
  `;
}

function sortCapabilities(items) {
  const priority = (item) => item.human_action_required ? 0 : item.ready ? 1 : 2;
  return items.slice().sort((left, right) => priority(left) - priority(right)
    || String(left.name).localeCompare(String(right.name)));
}

function shortHash(hash) {
  if (!hash) return '—';
  return String(hash).slice(0, 12);
}

function statusLabel(status) {
  return ({
    operational: 'prête',
    pending_approval: 'à approuver',
    update_pending_approval: 'mise à jour à approuver',
    invalid: 'invalide',
    invalid_update_retained: 'actif · mise à jour invalide',
    disabled: 'désactivée',
    rejected: 'refusée',
    update_rejected: 'mise à jour refusée',
  })[status] || status;
}

function statusTone(status) {
  if (status === 'operational' || status === 'invalid_update_retained') return 'status-done';
  if (status === 'pending_approval' || status === 'update_pending_approval') return 'status-review';
  return 'status-cancelled';
}

function runTone(status) {
  if (status === 'succeeded') return 'status-done';
  if (status === 'running' || status === 'waiting_decision') return 'status-review';
  if (status === 'failed') return 'status-blocked';
  return 'status-paused';
}
