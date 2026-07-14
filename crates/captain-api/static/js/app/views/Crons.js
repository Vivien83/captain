import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

function scheduleLabel(job) {
  const s = job.schedule || {};
  if (s.kind === 'cron' || s.Cron) {
    const expr = s.expr || (s.Cron && s.Cron.expr) || '';
    const tz = s.tz || (s.Cron && s.Cron.tz);
    return `${expr}${tz ? ` (${tz})` : ''}`;
  }
  if (s.kind === 'every' || s.Every) return `toutes les ${s.every_secs || (s.Every && s.Every.every_secs) || '?'}s`;
  if (s.kind === 'at' || s.At) return `à ${s.at || (s.At && s.At.at) || '?'}`;
  return JSON.stringify(s);
}

function actionLabel(job) {
  const a = job.action || {};
  if (a.kind === 'agent_turn' || a.AgentTurn) return (a.message || (a.AgentTurn && a.AgentTurn.message) || '').slice(0, 80);
  if (a.kind === 'system_event' || a.SystemEvent) return `événement système : ${a.text || ''}`;
  if (a.kind === 'workflow_run') return `workflow ${a.workflow_id || ''}`;
  return a.kind || 'action';
}

function deliveryLabel(job) {
  const d = job.delivery || {};
  if (d.kind === 'none' || !d.kind) return 'aucune livraison';
  if (d.kind === 'last_channel') return 'dernier canal utilisé';
  if (d.kind === 'channel') return `canal ${d.channel || ''} → ${d.to || ''}`;
  if (d.kind === 'webhook') return `webhook ${d.url || ''}`;
  return d.kind;
}

export function Crons() {
  const [agents, setAgents] = useState([]);
  const [jobs, setJobs] = useState(null);
  const [showForm, setShowForm] = useState(false);
  const [statusFor, setStatusFor] = useState(null);
  const [statusData, setStatusData] = useState(null);
  const [busyId, setBusyId] = useState(null);

  const load = useCallback(async () => {
    try {
      const [a, j] = await Promise.all([api.agents(), api.cronJobs()]);
      setAgents(a.agents || a || []);
      setJobs((j && j.jobs) || []);
    } catch (e) {
      toast(`Chargement impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const agentName = (id) => (agents.find((a) => a.id === id) || {}).name || id;

  const run = async (id) => {
    setBusyId(id);
    try {
      const res = await api.runCronJob(id);
      toast(`Exécuté : ${res.status || 'ok'}`);
    } catch (e) {
      toast(`Exécution impossible : ${e.message}`, 'err');
    } finally {
      setBusyId(null);
    }
  };

  const toggle = async (job) => {
    try { await api.toggleCronJob(job.id, job.enabled === false); await load(); }
    catch (e) { toast(`Action impossible : ${e.message}`, 'err'); }
  };

  const remove = async (id) => {
    try { await api.deleteCronJob(id); toast('Job supprimé'); await load(); }
    catch (e) { toast(`Suppression impossible : ${e.message}`, 'err'); }
  };

  const create = async (body) => {
    try { await api.createCronJob(body); setShowForm(false); toast('Job créé'); await load(); }
    catch (e) { toast(`Création impossible : ${e.message}`, 'err'); }
  };

  const showStatus = async (id) => {
    if (statusFor === id) { setStatusFor(null); return; }
    setStatusFor(id);
    setStatusData(null);
    try { setStatusData(await api.cronJobStatus(id)); }
    catch (e) { toast(`Statut indisponible : ${e.message}`, 'err'); }
  };

  // Rendue dans le hub Automation (onglet « Crons ») : le hub fournit
  // le wrapper .page et le titre, cette vue ne rend que son contenu.
  return html`
    <div>
        <p class="page-sub">Tâches planifiées exécutées automatiquement par Captain.</p>

        <div class="task-toolbar">
          <span class="spacer"></span>
          <button class="primary" onClick=${() => setShowForm((s) => !s)}>+ Nouveau job</button>
        </div>
        ${showForm && html`<${CronForm} agents=${agents} onCreate=${create} onCancel=${() => setShowForm(false)} />`}

        ${jobs === null && html`<div class="skeleton" style="height:70px"></div>`}
        ${jobs && jobs.length === 0 && html`<div class="empty-state"><div class="glyph">⏱️</div><div>Aucun job planifié.</div></div>`}
        ${jobs && jobs.length > 0 && html`
          <div class="item-list">
            ${jobs.map((job) => html`
              <div key=${job.id}>
                <div class="item-row">
                  <div class="item-row-main">
                    <strong>${job.name || job.id}</strong>
                    <span class="item-row-meta">${agentName(job.agent_id)} · ${scheduleLabel(job)}</span>
                    <span class="item-row-meta">${actionLabel(job)} · livraison : ${deliveryLabel(job)}</span>
                    ${job.consecutive_errors > 0 && html`<span class="item-row-meta" style="color:var(--err)">${job.consecutive_errors} échec(s) consécutif(s)</span>`}
                  </div>
                  <div class="item-row-actions">
                    <span class="status-pill ${job.enabled !== false ? 'status-done' : 'status-cancelled'}">${job.enabled !== false ? 'actif' : 'inactif'}</span>
                    <button class="ghost" disabled=${busyId === job.id} onClick=${() => run(job.id)}>Lancer</button>
                    <button class="ghost" onClick=${() => showStatus(job.id)}>${statusFor === job.id ? 'Masquer' : 'Statut'}</button>
                    <button class="ghost" onClick=${() => toggle(job)}>${job.enabled !== false ? 'Désactiver' : 'Activer'}</button>
                    <${DeleteButton} onConfirm=${() => remove(job.id)} />
                  </div>
                </div>
                ${statusFor === job.id && html`
                  <div class="detail-panel">
                    ${statusData ? html`<pre class="code-block">${JSON.stringify(statusData, null, 2)}</pre>` : html`<div class="skeleton" style="height:40px"></div>`}
                  </div>
                `}
              </div>
            `)}
          </div>
        `}
    </div>
  `;
}

function DeleteButton({ onConfirm }) {
  const [confirm, setConfirm] = useState(false);
  return html`
    <button class="ghost danger" style=${confirm ? 'color:var(--err)' : ''}
      onClick=${() => {
        if (!confirm) { setConfirm(true); setTimeout(() => setConfirm(false), 3000); return; }
        onConfirm();
      }}>${confirm ? 'Confirmer ?' : 'Supprimer'}</button>
  `;
}

function CronForm({ agents, onCreate, onCancel }) {
  const [agentId, setAgentId] = useState('');
  const [name, setName] = useState('');
  const [expr, setExpr] = useState('0 9 * * *');
  const [tz, setTz] = useState('Europe/Paris');
  const [message, setMessage] = useState('');

  const submit = (e) => {
    e.preventDefault();
    if (!agentId || !name.trim() || !message.trim()) return;
    onCreate({
      agent_id: agentId,
      name: name.trim(),
      schedule: { kind: 'cron', expr: expr.trim(), tz: tz.trim() || undefined },
      action: { kind: 'agent_turn', message: message.trim(), timeout_secs: 120 },
      delivery: { kind: 'last_channel' },
    });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <select value=${agentId} onChange=${(e) => setAgentId(e.target.value)}>
        <option value="">Agent…</option>
        ${agents.map((a) => html`<option value=${a.id}>${a.name}</option>`)}
      </select>
      <input type="text" placeholder="Nom" value=${name} onInput=${(e) => setName(e.target.value)} style="width:140px" />
      <input type="text" placeholder="Cron (5 champs)" value=${expr} onInput=${(e) => setExpr(e.target.value)} style="width:130px" />
      <input type="text" placeholder="Fuseau horaire" value=${tz} onInput=${(e) => setTz(e.target.value)} style="width:130px" />
      <input type="text" placeholder="Message à envoyer" value=${message}
        onInput=${(e) => setMessage(e.target.value)} style="flex:1" />
      <button class="primary" type="submit">Créer</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}
