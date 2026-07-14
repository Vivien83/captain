import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { getState, setState, subscribe, toast } from '../store.js';
import { PRIMARY_HUBS, hubForRoute } from '../control_contract.mjs';

const html = htm.bind(h);

export function Shell({ route, children }) {
  const [st, setSt] = useState(getState());
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [modelUpdates, setModelUpdates] = useState(null);
  useEffect(() => subscribe((s) => setSt({ ...s })), []);

  // Agents + daemon status, refreshed lazily.
  useEffect(() => {
    const load = async () => {
      try {
        const [agents, status] = await Promise.all([api.agents(), api.status()]);
        const list = agents.agents || agents || [];
        const patch = { agents: list, daemon: { ok: true, version: status.version || '' } };
        if (!getState().currentAgentId && list.length) {
          const captain = list.find((a) => a.name === 'captain') || list[0];
          patch.currentAgentId = captain.id;
        }
        setState(patch);
      } catch {
        setState({ daemon: { ok: false, version: '' } });
      }
    };
    load();
    const t = setInterval(load, 15000);
    return () => clearInterval(t);
  }, []);

  // Durable Codex catalog additions stay visible until the user decides.
  useEffect(() => {
    const load = async () => {
      try { setModelUpdates(await api.modelUpdates()); } catch { /* transient */ }
    };
    load();
    const t = setInterval(load, 60000);
    return () => clearInterval(t);
  }, []);

  // Approvals badge poll (cheap; SSE nudges refresh it faster elsewhere).
  useEffect(() => {
    const load = async () => {
      try {
        const res = await api.approvals();
        setState({ approvalsCount: (res.approvals || []).length });
      } catch { /* daemon briefly away — badge keeps last value */ }
    };
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, []);

  // Sessions of the current agent.
  useEffect(() => {
    if (!st.currentAgentId) return;
    refreshSessions(st.currentAgentId);
  }, [st.currentAgentId]);

  const nav = (hash) => { location.hash = hash; setDrawerOpen(false); };

  return html`
    <div class="shell">
      ${drawerOpen && html`<div class="scrim" onClick=${() => setDrawerOpen(false)}></div>`}
      <div class="sidebar ${drawerOpen ? 'open' : ''}">
        <div class="sidebar-brand">
          <img src="/assets/logo.png?rev=wordmark-2" alt="" />
          <span class="name">Captain</span>
        </div>
        <div class="sidebar-nav">
          ${PRIMARY_HUBS.map((item) => html`
            <a key=${item.route} class="nav-item ${hubForRoute(route) === item.route ? 'active' : ''}"
              href="#/${item.route}" onClick=${() => setDrawerOpen(false)}>
              <span>${item.icon}</span> ${item.label}
              ${item.route === 'automation' && st.approvalsCount > 0 && html`<span class="badge">${st.approvalsCount}</span>`}
            </a>
          `)}
        </div>

        <div class="sidebar-section-title">
          Sessions
          <button class="ghost" title="Nouvelle session" onClick=${() => newSession(st.currentAgentId)}>+</button>
        </div>
        <div class="session-list">
          ${st.sessions.map((s) => html`
            <${SessionRow} key=${s.session_id} session=${s} agentId=${st.currentAgentId}
              active=${s.session_id === st.currentSessionId} />
          `)}
          ${st.sessions.length === 0 && html`<div class="session-item" style="color:var(--text-2)">Aucune session</div>`}
        </div>

        <div class="sidebar-footer">
          <span class="status-dot ${st.daemon.ok === false ? 'err' : ''}"></span>
          <span>${st.daemon.ok === false ? 'daemon hors ligne' : (st.daemon.version || 'connecté')}</span>
          <a href="/terminal" title="Mode expert (terminal)">⌥ expert</a>
        </div>
      </div>

      <div class="main">
        <div class="topbar">
          <button class="ghost menu-btn" onClick=${() => setDrawerOpen(true)}>☰</button>
          <span class="title">${(PRIMARY_HUBS.find((it) => it.route === hubForRoute(route)) || PRIMARY_HUBS[0]).label}</span>
          ${st.backgroundActivity.length > 0 && html`
            <span class="bg-activity"><span class="spinner"></span> ${st.backgroundActivity.length} en arrière-plan</span>
          `}
          <span class="spacer"></span>
          ${route === 'chat' && st.agents.length > 1 && html`
            <select value=${st.currentAgentId || ''}
              onChange=${(e) => setState({ currentAgentId: e.target.value, currentSessionId: null })}>
              ${st.agents.map((a) => html`<option value=${a.id}>${a.name}</option>`)}
            </select>
          `}
        </div>
        ${modelUpdates && modelUpdates.pending && modelUpdates.pending.length > 0 && html`
          <${ModelUpdateNotice} snapshot=${modelUpdates} onRefresh=${async () => setModelUpdates(await api.modelUpdates())} />
        `}
        ${children}
      </div>

      <div class="toasts">
        ${st.toasts.map((t) => html`<div class="toast ${t.kind}" key=${t.id}>${t.text}</div>`)}
      </div>
    </div>
  `;
}

function ModelUpdateNotice({ snapshot, onRefresh }) {
  const update = snapshot.pending[0];
  const [agentId, setAgentId] = useState((snapshot.agents[0] || {}).agent_id || '');
  const [choosingStrategy, setChoosingStrategy] = useState(false);
  const [busy, setBusy] = useState(false);
  const more = snapshot.pending.length - 1;
  const targetAgent = snapshot.agents.find((agent) => agent.agent_id === agentId);

  useEffect(() => setChoosingStrategy(false), [update.model_id]);

  useEffect(() => {
    if (!snapshot.agents.some((agent) => agent.agent_id === agentId)) {
      setAgentId((snapshot.agents[0] || {}).agent_id || '');
    }
  }, [snapshot, agentId]);

  const decide = async (decision, sessionStrategy) => {
    setBusy(true);
    try {
      const body = { model_id: update.model_id, decision };
      if (decision === 'switch') {
        body.agent_id = agentId;
        body.session_strategy = sessionStrategy;
      }
      const result = await api.decideModelUpdate(body);
      toast(decision === 'keep' ? 'Modèle actuel conservé' : (result.message || 'Modèle Codex mis à jour'));
      try { await onRefresh(); } catch { /* decision succeeded; next poll refreshes the banner */ }
    } catch (error) {
      toast(`Décision impossible : ${error.message}`, 'err');
    } finally {
      setBusy(false);
    }
  };

  return html`
    <div class="model-update-notice" role="region" aria-label="Mise à jour du modèle Codex">
      <div class="model-update-copy" aria-live="polite">
        <strong>Nouveau modèle Codex</strong>
        <span>${update.display_name}</span>
        <code>${update.model_id}</code>
        ${snapshot.agents.length === 1 && targetAgent && html`
          <span class="model-update-current">Actuel : ${targetAgent.current_model}</span>
        `}
        ${more > 0 && html`<span class="model-update-more">+${more} autre${more > 1 ? 's' : ''}</span>`}
      </div>
      <div class="model-update-actions">
        ${snapshot.agents.length > 1 && html`
          <select value=${agentId} disabled=${busy} aria-label="Agent à mettre à jour"
            onChange=${(event) => setAgentId(event.target.value)}>
            ${snapshot.agents.map((agent) => html`
              <option value=${agent.agent_id}>${agent.agent_name} · ${agent.current_model}</option>
            `)}
          </select>
        `}
        ${choosingStrategy ? html`
          <button disabled=${busy || !agentId} onClick=${() => decide('switch', 'new_session')}>Nouvelle session</button>
          <button class="primary" disabled=${busy || !agentId} onClick=${() => decide('switch', 'compact_session')}>Résumé compact</button>
          <button class="ghost" disabled=${busy} title="Retour" aria-label="Retour"
            onClick=${() => setChoosingStrategy(false)}>←</button>
        ` : html`
          <button disabled=${busy} onClick=${() => decide('keep')}>Conserver</button>
          <button class="primary" disabled=${busy || !agentId} onClick=${() => setChoosingStrategy(true)}>Basculer</button>
        `}
      </div>
    </div>
  `;
}

const shortId = (id) => (id || '').slice(0, 8);

async function refreshSessions(agentId) {
  try {
    const res = await api.agentSessions(agentId);
    const sessions = res.sessions || res || [];
    const active = sessions.find((s) => s.active);
    setState({
      sessions,
      currentSessionId: active ? active.session_id : getState().currentSessionId,
    });
  } catch { /* transient */ }
}

async function newSession(agentId) {
  if (!agentId) return;
  try {
    await api.createSession(agentId);
    await refreshSessions(agentId);
    // Reload chat by re-selecting the agent (Chat listens to currentAgentId).
    const cur = getState().currentAgentId;
    setState({ currentAgentId: null });
    setState({ currentAgentId: cur });
    toast('Nouvelle session créée');
  } catch (e) {
    toast(`Création impossible : ${e.message}`, 'err');
  }
}

async function switchTo(agentId, session) {
  try {
    await api.switchSession(agentId, session.session_id);
    setState({ currentSessionId: session.session_id });
    const cur = getState().currentAgentId;
    setState({ currentAgentId: null });
    setState({ currentAgentId: cur });
  } catch (e) {
    toast(`Bascule impossible : ${e.message}`, 'err');
  }
}

// No blocking browser dialogs: inline rename input, two-click delete confirm.
function SessionRow({ session, agentId, active }) {
  const [editing, setEditing] = useState(false);
  const [label, setLabel] = useState(session.label || '');
  const [confirmDel, setConfirmDel] = useState(false);

  const saveLabel = async () => {
    setEditing(false);
    if ((label || '') === (session.label || '')) return;
    try {
      await api.labelSession(session.session_id, label);
      await refreshSessions(agentId);
    } catch (e) {
      toast(`Renommage impossible : ${e.message}`, 'err');
    }
  };

  const del = async (e) => {
    e.stopPropagation();
    if (!confirmDel) {
      setConfirmDel(true);
      setTimeout(() => setConfirmDel(false), 3000);
      return;
    }
    try {
      await api.deleteSession(session.session_id);
      await refreshSessions(agentId);
      toast('Session supprimée');
    } catch (err) {
      toast(`Suppression impossible : ${err.message}`, 'err');
    }
  };

  return html`
    <div class="session-item ${active ? 'active' : ''}"
      onClick=${() => !editing && switchTo(agentId, session)}>
      ${editing
        ? html`<input type="text" value=${label} style="flex:1;font-size:13px;padding:3px 8px"
            onClick=${(e) => e.stopPropagation()}
            onInput=${(e) => setLabel(e.target.value)}
            onKeyDown=${(e) => { if (e.key === 'Enter') saveLabel(); if (e.key === 'Escape') setEditing(false); }}
            onBlur=${saveLabel} autofocus />`
        : html`<span class="label">${session.label || shortId(session.session_id)}</span>`}
      <span class="actions">
        <button title="Renommer" onClick=${(e) => { e.stopPropagation(); setLabel(session.label || ''); setEditing(true); }}>✎</button>
        <button title=${confirmDel ? 'Confirmer la suppression' : 'Supprimer'}
          style=${confirmDel ? 'color:var(--err)' : ''} onClick=${del}>${confirmDel ? '✓?' : '🗑'}</button>
      </span>
    </div>
  `;
}
