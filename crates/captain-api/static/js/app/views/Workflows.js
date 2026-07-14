import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

export function Workflows() {
  const [workflows, setWorkflows] = useState(null);
  const [agents, setAgents] = useState([]);
  const [expandedId, setExpandedId] = useState(null);
  const [runs, setRuns] = useState({});
  const [inputs, setInputs] = useState({});
  const [lastOutputs, setLastOutputs] = useState({});
  const [busyId, setBusyId] = useState(null);
  const [showForm, setShowForm] = useState(false);

  const load = useCallback(async () => {
    try {
      const [workflowData, agentData] = await Promise.all([api.workflows(), api.agents()]);
      setWorkflows(workflowData || []);
      setAgents((agentData && agentData.agents) || agentData || []);
    } catch (e) {
      toast(`Chargement impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const loadRuns = async (id) => {
    try {
      const history = await api.workflowRuns(id);
      setRuns((current) => ({ ...current, [id]: history || [] }));
    } catch (e) {
      toast(`Historique indisponible : ${e.message}`, 'err');
    }
  };

  const toggleDetails = async (id) => {
    if (expandedId === id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(id);
    await loadRuns(id);
  };

  const run = async (id) => {
    setBusyId(id);
    try {
      const result = await api.runWorkflow(id, inputs[id] || '');
      setLastOutputs((current) => ({ ...current, [id]: result.output || '' }));
      toast('Workflow terminé');
      await loadRuns(id);
    } catch (e) {
      toast(`Exécution impossible : ${e.message}`, 'err');
    } finally {
      setBusyId(null);
    }
  };

  const create = async (body) => {
    try {
      await api.createWorkflow(body);
      setShowForm(false);
      toast('Workflow créé');
      await load();
    } catch (e) {
      toast(`Création impossible : ${e.message}`, 'err');
    }
  };

  const remove = async (id) => {
    try {
      await api.deleteWorkflow(id);
      if (expandedId === id) setExpandedId(null);
      toast('Workflow supprimé');
      await load();
    } catch (e) {
      toast(`Suppression impossible : ${e.message}`, 'err');
    }
  };

  return html`
    <div>
      <div class="task-toolbar">
        <p class="page-sub" style="margin:0">Pipelines persistants exécutés par les agents Captain.</p>
        <span class="spacer"></span>
        <button class="primary" onClick=${() => setShowForm((value) => !value)}>+ Workflow</button>
      </div>

      ${showForm && html`<${WorkflowForm} agents=${agents} onCreate=${create} onCancel=${() => setShowForm(false)} />`}
      ${workflows === null && html`<div class="skeleton" style="height:70px"></div>`}
      ${workflows && workflows.length === 0 && html`
        <div class="empty-state"><div class="glyph">⇢</div><div>Aucun workflow.</div></div>
      `}
      ${workflows && workflows.length > 0 && html`
        <div class="item-list">
          ${workflows.map((workflow) => html`
            <div class="workflow-entry" key=${workflow.id}>
              <div class="item-row">
                <div class="item-row-main">
                  <strong>${workflow.name || workflow.id}</strong>
                  ${workflow.description && html`<span class="item-row-meta">${workflow.description}</span>`}
                  <span class="item-row-meta">${workflow.steps || 0} étape(s) · créé ${formatDate(workflow.created_at)}</span>
                </div>
                <div class="item-row-actions">
                  <button class="ghost" onClick=${() => toggleDetails(workflow.id)}>${expandedId === workflow.id ? 'Fermer' : 'Ouvrir'}</button>
                  <${DeleteButton} onConfirm=${() => remove(workflow.id)} />
                </div>
              </div>
              ${expandedId === workflow.id && html`
                <div class="workflow-detail">
                  <div class="workflow-run-form">
                    <textarea rows="3" placeholder="Entrée du workflow"
                      value=${inputs[workflow.id] || ''}
                      onInput=${(event) => setInputs((current) => ({ ...current, [workflow.id]: event.target.value }))}></textarea>
                    <button class="primary" disabled=${busyId === workflow.id} onClick=${() => run(workflow.id)}>
                      ${busyId === workflow.id ? 'Exécution…' : 'Exécuter'}
                    </button>
                  </div>
                  ${lastOutputs[workflow.id] && html`
                    <div class="workflow-output">
                      <span class="status-label">Dernière sortie</span>
                      <pre class="code-block">${lastOutputs[workflow.id]}</pre>
                    </div>
                  `}
                  <h3 class="status-label">Historique</h3>
                  ${runs[workflow.id] === undefined && html`<div class="skeleton" style="height:42px"></div>`}
                  ${runs[workflow.id] && runs[workflow.id].length === 0 && html`<div class="status-empty">Aucune exécution.</div>`}
                  ${runs[workflow.id] && runs[workflow.id].length > 0 && html`
                    <div class="workflow-runs">
                      ${runs[workflow.id].slice(0, 8).map((runItem) => html`
                        <div class="workflow-run" key=${runItem.id}>
                          <span class="status-pill ${runStateClass(runItem.state)}">${runItem.state || 'unknown'}</span>
                          <span>${formatDate(runItem.started_at)}</span>
                          <span>${runItem.steps_completed || 0} étape(s)</span>
                          ${runItem.error && html`<span class="workflow-run-error">${runItem.error}</span>`}
                        </div>
                      `)}
                    </div>
                  `}
                </div>
              `}
            </div>
          `)}
        </div>
      `}
    </div>
  `;
}

function WorkflowForm({ agents, onCreate, onCancel }) {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [agentId, setAgentId] = useState('');
  const [prompt, setPrompt] = useState('Traite cette demande : {{input}}');
  const [timeoutSecs, setTimeoutSecs] = useState('120');

  const submit = (event) => {
    event.preventDefault();
    if (!name.trim() || !agentId || !prompt.trim()) return;
    onCreate({
      name: name.trim(),
      description: description.trim(),
      steps: [{
        name: 'run',
        agent_id: agentId,
        prompt: prompt.trim(),
        mode: 'sequential',
        timeout_secs: Number(timeoutSecs) || 120,
        error_mode: 'fail',
        output_var: 'result',
      }],
    });
  };

  return html`
    <form class="workflow-create-form" onSubmit=${submit}>
      <div class="workflow-form-grid">
        <input type="text" placeholder="Nom" value=${name} onInput=${(event) => setName(event.target.value)} />
        <select value=${agentId} onChange=${(event) => setAgentId(event.target.value)}>
          <option value="">Agent…</option>
          ${agents.map((agent) => html`<option value=${agent.id}>${agent.name}</option>`)}
        </select>
        <input type="number" min="1" max="3600" title="Timeout en secondes"
          value=${timeoutSecs} onInput=${(event) => setTimeoutSecs(event.target.value)} />
      </div>
      <input type="text" placeholder="Description" value=${description}
        onInput=${(event) => setDescription(event.target.value)} />
      <textarea rows="3" placeholder="Prompt" value=${prompt}
        onInput=${(event) => setPrompt(event.target.value)}></textarea>
      <div class="form-actions">
        <button class="primary" type="submit">Créer</button>
        <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
      </div>
    </form>
  `;
}

function DeleteButton({ onConfirm }) {
  const [confirm, setConfirm] = useState(false);
  return html`
    <button class="ghost danger" onClick=${() => {
      if (!confirm) {
        setConfirm(true);
        setTimeout(() => setConfirm(false), 3000);
        return;
      }
      onConfirm();
    }}>${confirm ? 'Confirmer ?' : 'Supprimer'}</button>
  `;
}

function runStateClass(state) {
  if (state === 'completed') return 'status-done';
  if (state === 'running' || state === 'pending') return 'status-active';
  return 'status-blocked';
}

function formatDate(value) {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' });
}
