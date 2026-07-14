import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { setState, toast } from '../store.js';

const html = htm.bind(h);

export function Approvals() {
  const [items, setItems] = useState(null); // null = loading
  const [busyId, setBusyId] = useState(null);

  const load = async () => {
    try {
      const res = await api.approvals();
      const list = res.approvals || [];
      setItems(list);
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

  // Rendue dans le hub Automation (onglet « Approbations ») : le hub
  // fournit le wrapper .page et le titre, cette vue ne rend que son contenu.
  return html`
    <div>
        <p class="page-sub">Chaque action sensible attend ta décision — rien ne s'exécute sans toi.</p>

        ${items === null && html`
          <div class="skeleton" style="height:110px;margin-bottom:14px"></div>
          <div class="skeleton" style="height:110px"></div>
        `}

        ${items && items.length === 0 && html`
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
                onClick=${() => act(a.id, api.approve, 'Approuvé')}>Approuver</button>
              <button disabled=${busyId === a.id}
                onClick=${() => act(a.id, api.approveSession, 'Approuvé pour la session')}>Pour la session</button>
              <button class="danger" disabled=${busyId === a.id}
                onClick=${() => act(a.id, api.reject, 'Refusé')}>Refuser</button>
            </div>
          </div>
        `)}
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
