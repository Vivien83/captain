import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

function patternLabel(p) {
  if (typeof p === 'string') return p;
  if (p && typeof p === 'object') {
    const key = Object.keys(p)[0];
    const val = p[key] || {};
    if (key === 'content_match') return `contenu contient "${val.substring || ''}"`;
    if (key === 'channel_message') return `canal ${val.channel || '*'}${val.contains ? ` contient "${val.contains}"` : ''}`;
    if (key === 'system_keyword') return `système : "${val.keyword || ''}"`;
    if (key === 'agent_spawned') return `agent démarré : ${val.name_pattern || ''}`;
    if (key === 'memory_key_pattern') return `mémoire : ${val.key_pattern || ''}`;
    return key;
  }
  return String(p);
}

function buildPattern(type, value) {
  if (type === 'all' || type === 'system') return type;
  if (type === 'content_match') return { content_match: { substring: value } };
  if (type === 'channel_message') {
    const [channel, contains] = value.split(':');
    return { channel_message: { channel: (channel || '').trim(), contains: (contains || '').trim() } };
  }
  return 'all';
}

export function Triggers() {
  const [agents, setAgents] = useState([]);
  const [triggers, setTriggers] = useState(null);
  const [fileTriggers, setFileTriggers] = useState(null);
  const [showEventForm, setShowEventForm] = useState(false);
  const [showFileForm, setShowFileForm] = useState(false);

  const load = useCallback(async () => {
    try {
      const [a, t, f] = await Promise.all([
        api.agents(), api.triggers(), api.fileTriggers(),
      ]);
      setAgents(a.agents || a || []);
      setTriggers(t || []);
      setFileTriggers(f || []);
    } catch (e) {
      toast(`Chargement impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => { load(); }, [load]);

  const agentName = (id) => (agents.find((a) => a.id === id) || {}).name || id;

  const toggleTrigger = async (t) => {
    try { await api.updateTrigger(t.id, { enabled: !t.enabled }); await load(); }
    catch (e) { toast(`Action impossible : ${e.message}`, 'err'); }
  };
  const deleteTrigger = async (id) => {
    try { await api.deleteTrigger(id); toast('Trigger supprimé'); await load(); }
    catch (e) { toast(`Suppression impossible : ${e.message}`, 'err'); }
  };
  const createTrigger = async (body) => {
    try { await api.createTrigger(body); setShowEventForm(false); toast('Trigger créé'); await load(); }
    catch (e) { toast(`Création impossible : ${e.message}`, 'err'); }
  };

  const toggleFileTrigger = async (t) => {
    try { await api.updateFileTrigger(t.id, { enabled: !t.enabled }); await load(); }
    catch (e) { toast(`Action impossible : ${e.message}`, 'err'); }
  };
  const deleteFileTrigger = async (id) => {
    try { await api.deleteFileTrigger(id); toast('Trigger fichier supprimé'); await load(); }
    catch (e) { toast(`Suppression impossible : ${e.message}`, 'err'); }
  };
  const createFileTrigger = async (body) => {
    try { await api.createFileTrigger(body); setShowFileForm(false); toast('Trigger fichier créé'); await load(); }
    catch (e) { toast(`Création impossible : ${e.message}`, 'err'); }
  };

  // Rendue dans le hub Automation (onglet « Triggers ») : le hub fournit
  // le wrapper .page et le titre, cette vue ne rend que son contenu.
  return html`
    <div>
        <p class="page-sub">Réveille un agent sur un événement ou un fichier modifié.</p>

        <div class="task-toolbar">
          <h2 class="section-title" style="margin:0">Événements</h2>
          <span class="spacer"></span>
          <button class="primary" onClick=${() => setShowEventForm((s) => !s)}>+ Trigger</button>
        </div>
        ${showEventForm && html`<${EventTriggerForm} agents=${agents} onCreate=${createTrigger} onCancel=${() => setShowEventForm(false)} />`}
        ${triggers === null && html`<div class="skeleton" style="height:60px"></div>`}
        ${triggers && triggers.length === 0 && html`<div class="empty-state"><div class="glyph">⚡</div><div>Aucun trigger d'événement.</div></div>`}
        ${triggers && triggers.length > 0 && html`
          <div class="item-list">
            ${triggers.map((t) => html`
              <div class="item-row" key=${t.id}>
                <div class="item-row-main">
                  <strong>${agentName(t.agent_id)}</strong>
                  <span class="item-row-meta">${patternLabel(t.pattern)} ${t.max_fires ? `· max ${t.max_fires} déclenchements` : ''}</span>
                </div>
                <div class="item-row-actions">
                  <span class="status-pill ${t.enabled !== false ? 'status-done' : 'status-cancelled'}">${t.enabled !== false ? 'actif' : 'inactif'}</span>
                  <button class="ghost" onClick=${() => toggleTrigger(t)}>${t.enabled !== false ? 'Désactiver' : 'Activer'}</button>
                  <${DeleteButton} onConfirm=${() => deleteTrigger(t.id)} />
                </div>
              </div>
            `)}
          </div>
        `}

        <div class="task-toolbar" style="margin-top:26px">
          <h2 class="section-title" style="margin:0">Fichiers</h2>
          <span class="spacer"></span>
          <button class="primary" onClick=${() => setShowFileForm((s) => !s)}>+ Trigger fichier</button>
        </div>
        ${showFileForm && html`<${FileTriggerForm} agents=${agents} onCreate=${createFileTrigger} onCancel=${() => setShowFileForm(false)} />`}
        ${fileTriggers === null && html`<div class="skeleton" style="height:60px"></div>`}
        ${fileTriggers && fileTriggers.length === 0 && html`<div class="empty-state"><div class="glyph">📁</div><div>Aucun trigger fichier.</div></div>`}
        ${fileTriggers && fileTriggers.length > 0 && html`
          <div class="item-list">
            ${fileTriggers.map((t) => html`
              <div class="item-row" key=${t.id}>
                <div class="item-row-main">
                  <strong>${agentName(t.agent_id)}</strong>
                  <span class="item-row-meta">${(t.paths || []).join(', ')} · ${(t.events || []).join('/')}</span>
                </div>
                <div class="item-row-actions">
                  <span class="status-pill ${t.enabled !== false ? 'status-done' : 'status-cancelled'}">${t.enabled !== false ? 'actif' : 'inactif'}</span>
                  <button class="ghost" onClick=${() => toggleFileTrigger(t)}>${t.enabled !== false ? 'Désactiver' : 'Activer'}</button>
                  <${DeleteButton} onConfirm=${() => deleteFileTrigger(t.id)} />
                </div>
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

function AgentSelect({ agents, value, onChange }) {
  return html`
    <select value=${value} onChange=${(e) => onChange(e.target.value)}>
      <option value="">Agent…</option>
      ${agents.map((a) => html`<option value=${a.id}>${a.name}</option>`)}
    </select>
  `;
}

function EventTriggerForm({ agents, onCreate, onCancel }) {
  const [agentId, setAgentId] = useState('');
  const [type, setType] = useState('all');
  const [value, setValue] = useState('');
  const [prompt, setPrompt] = useState('');
  const [maxFires, setMaxFires] = useState('');

  const submit = (e) => {
    e.preventDefault();
    if (!agentId || !prompt.trim()) return;
    onCreate({
      agent_id: agentId,
      pattern: buildPattern(type, value),
      prompt_template: prompt.trim(),
      max_fires: maxFires ? Number(maxFires) : null,
    });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <${AgentSelect} agents=${agents} value=${agentId} onChange=${setAgentId} />
      <select value=${type} onChange=${(e) => setType(e.target.value)}>
        <option value="all">tout événement</option>
        <option value="system">tout événement système</option>
        <option value="content_match">contenu contient…</option>
        <option value="channel_message">message sur un canal</option>
      </select>
      ${(type === 'content_match' || type === 'channel_message') && html`
        <input type="text" style="width:180px"
          placeholder=${type === 'channel_message' ? 'canal:texte (optionnel)' : 'texte à repérer'}
          value=${value} onInput=${(e) => setValue(e.target.value)} />
      `}
      <input type="text" placeholder="Message à envoyer à l'agent" value=${prompt}
        onInput=${(e) => setPrompt(e.target.value)} style="flex:1" />
      <input type="number" placeholder="max" title="Nombre max de déclenchements" value=${maxFires}
        onInput=${(e) => setMaxFires(e.target.value)} style="width:70px" />
      <button class="primary" type="submit">Créer</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}

function FileTriggerForm({ agents, onCreate, onCancel }) {
  const [agentId, setAgentId] = useState('');
  const [path, setPath] = useState('');
  const [event, setEvent] = useState('modify');
  const [prompt, setPrompt] = useState('');

  const submit = (e) => {
    e.preventDefault();
    if (!agentId || !path.trim()) return;
    onCreate({
      agent_id: agentId,
      paths: [path.trim()],
      events: [event],
      prompt_template: prompt.trim() || `Le fichier ${path.trim()} a changé (${event}).`,
    });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <${AgentSelect} agents=${agents} value=${agentId} onChange=${setAgentId} />
      <input type="text" placeholder="Chemin à surveiller" value=${path}
        onInput=${(e) => setPath(e.target.value)} style="flex:1" />
      <select value=${event} onChange=${(e) => setEvent(e.target.value)}>
        <option value="create">création</option>
        <option value="modify">modification</option>
        <option value="delete">suppression</option>
      </select>
      <input type="text" placeholder="Message (optionnel)" value=${prompt}
        onInput=${(e) => setPrompt(e.target.value)} style="flex:1" />
      <button class="primary" type="submit">Créer</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}
