import { h } from '/assets/app/vendor/preact.module.js';
import { useState, useEffect } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api } from '../api.js';
import { setState, toast } from '../store.js';

const html = htm.bind(h);

export function Approvals() {
  const [items, setItems] = useState(null); // null = loading
  const [rules, setRules] = useState([]);
  const [reasons, setReasons] = useState({});
  const [busyId, setBusyId] = useState(null);

  const load = async () => {
    try {
      const res = await api.approvals();
      const list = res.approvals || [];
      setItems(list);
      setRules(res.rules || []);
      setState({ approvalsCount: list.length });
    } catch { /* transient — keep last view */ }
  };

  useEffect(() => {
    load();
    const t = setInterval(load, 3000);
    return () => clearInterval(t);
  }, []);

  const act = async (id, fn, label) => {
    setBusyId(id);
    try {
      await fn(id);
      toast(label);
      await load();
    } catch (e) {
      toast(`Action impossible : ${e.message}`, 'err');
    } finally {
      setBusyId(null);
    }
  };

  const reasonFor = (id) => (reasons[id] || '').trim();
  const setReason = (id, value) => setReasons((current) => ({ ...current, [id]: value }));

  // Rendue dans le hub Automation (onglet « Approbations ») : le hub
  // fournit le wrapper .page et le titre, cette vue ne rend que son contenu.
  return html`
    <div>
        <p class="page-sub">Chaque action sensible attend ta décision — rien ne s'exécute sans toi.</p>

        ${items === null && html`
          <div class="skeleton" style="height:110px;margin-bottom:14px"></div>
          <div class="skeleton" style="height:110px"></div>
        `}

        ${items && items.length === 0 && rules.length === 0 && html`
          <div class="empty-state">
            <div class="glyph">🛡️</div>
            <div>Aucune approbation en attente.</div>
            <div style="font-size:13px;margin-top:6px">Quand un agent voudra exécuter une action sensible, elle apparaîtra ici.</div>
          </div>
        `}

        ${items && items.map((a) => html`
          <div class="approval-card" key=${a.id}>
            <div class="meta">
              <span class="tool-chip">${a.tool_name}</span>
              <span>agent : ${a.agent_name || a.agent_id}</span>
              <span style="margin-left:auto">${timeAgo(a.requested_at)}</span>
            </div>
            <div class="summary">${a.action_summary || a.description || ''}</div>
            <div class="actions">
              <button class="primary" disabled=${busyId === a.id}
                onClick=${() => act(a.id, api.approve, 'Approuvé une fois')}>Une fois</button>
              <button disabled=${busyId === a.id}
                onClick=${() => act(a.id, api.approveSession, 'Autorisé pour cette session')}>Session</button>
              <button disabled=${busyId === a.id}
                onClick=${() => act(a.id, api.approveAlways, 'Règle exacte créée')}>Toujours cette action</button>
            </div>
            <div class="approval-reject">
              <label for=${`approval-reason-${a.id}`}>Motif transmis à l’agent</label>
              <input id=${`approval-reason-${a.id}`} type="text" maxlength="280"
                placeholder="Ex. utilise plutôt le serveur de test"
                value=${reasons[a.id] || ''}
                onInput=${(event) => setReason(a.id, event.currentTarget.value)} />
            </div>
            <div class="actions approval-deny-actions">
              <button class="danger" disabled=${busyId === a.id}
                onClick=${() => act(a.id, (id) => api.reject(id, reasonFor(id)), 'Refusé une fois')}>Refuser</button>
              <button class="danger" disabled=${busyId === a.id}
                onClick=${() => act(a.id, (id) => api.rejectSession(id, reasonFor(id)), 'Refusé pour cette session')}>Refuser (session)</button>
              <button class="danger" disabled=${busyId === a.id || !reasonFor(a.id)}
                title=${reasonFor(a.id) ? '' : 'Un motif est obligatoire pour une règle durable'}
                onClick=${() => act(a.id, (id) => api.rejectAlways(id, reasonFor(id)), 'Règle de blocage exacte créée')}>Bloquer cette action</button>
            </div>
          </div>
        `)}

        ${rules.length > 0 && html`
          <section class="approval-rules" aria-labelledby="approval-rules-title">
            <div class="approval-rules-heading">
              <div>
                <h3 id="approval-rules-title">Règles durables</h3>
                <p>Liées à un agent, un outil et l’empreinte exacte de l’action.</p>
              </div>
              <span>${rules.length}</span>
            </div>
            <div class="approval-rule-list">
              ${rules.map((rule) => html`
                <div class="approval-rule" key=${rule.id}>
                  <span class=${`rule-effect ${rule.effect}`}>${rule.effect === 'allow' ? 'Autoriser' : 'Bloquer'}</span>
                  <div class="rule-main">
                    <strong>${rule.tool_name}</strong>
                    <span>agent ${rule.agent_id} · action ${rule.action_digest.slice(0, 10)}</span>
                    ${rule.reason && html`<small>${rule.reason}</small>`}
                  </div>
                  <button class="ghost" disabled=${busyId === rule.id}
                    onClick=${() => act(rule.id, api.revokeApprovalRule, 'Règle révoquée')}>Révoquer</button>
                </div>
              `)}
            </div>
          </section>
        `}
    </div>
  `;
}

function timeAgo(iso) {
  if (!iso) return '';
  const s = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000);
  if (s < 60) return `il y a ${Math.floor(s)}s`;
  if (s < 3600) return `il y a ${Math.floor(s / 60)}min`;
  return `il y a ${Math.floor(s / 3600)}h`;
}
