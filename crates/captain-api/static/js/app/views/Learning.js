import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

export function Learning() {
  const [pending, setPending] = useState(null);
  const [committed, setCommitted] = useState(null);
  const [metrics, setMetrics] = useState(null);
  const [busyId, setBusyId] = useState(null);

  const load = useCallback(async () => {
    try {
      const [rev, com, met] = await Promise.all([
        api.learningReview(), api.learningCommitted(), api.learningMetrics(),
      ]);
      setPending(rev.pending || []);
      setCommitted(com.committed || []);
      setMetrics(met);
    } catch (e) {
      toast(`Chargement impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => {
    load();
    const t = setInterval(load, 8000);
    return () => clearInterval(t);
  }, [load]);

  const decide = async (id, approve) => {
    setBusyId(id);
    try {
      await api.learningDecide(id, approve);
      toast(approve ? 'Approuvé' : 'Refusé');
      await load();
    } catch (e) {
      toast(`Action impossible : ${e.message}`, 'err');
    } finally {
      setBusyId(null);
    }
  };

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Learning</h1>
        <p class="page-sub">Mémoire acquise automatiquement — chaque écriture passe par une révision avant d'être retenue.</p>

        ${metrics && html`
          <div class="metrics-row">
            <div class="metric-chip">${metrics.review_queue_pending ?? (pending || []).length} en attente</div>
            <div class="metric-chip">${(committed || []).length} retenues</div>
            <div class="metric-chip">mode : ${metrics.learning_mode || 'n/a'}</div>
            <div class="metric-chip ${metrics.learning_enabled ? 'ok' : 'off'}">${metrics.learning_enabled ? 'actif' : 'désactivé'}</div>
          </div>
        `}

        <h2 class="section-title">En attente de révision</h2>
        ${pending === null && html`<div class="skeleton" style="height:70px;margin-bottom:10px"></div>`}
        ${pending && pending.length === 0 && html`
          <div class="empty-state">
            <div class="glyph">🧠</div>
            <div>Rien à réviser pour l'instant.</div>
          </div>
        `}
        ${pending && pending.length > 0 && html`
          <div class="item-list">
            ${pending.map((it) => html`
              <div class="item-row" key=${it.id}>
                <div class="item-row-main">
                  <strong>${it.subject || '—'}</strong>
                  <span class="item-row-meta">${it.outcome || it.kind || ''} ${it.predicate ? `· ${it.predicate}` : ''} ${it.object ? `→ ${it.object}` : ''}</span>
                </div>
                <div class="item-row-actions">
                  <button class="primary" disabled=${busyId === it.id} onClick=${() => decide(it.id, true)}>Approuver</button>
                  <button class="danger" disabled=${busyId === it.id} onClick=${() => decide(it.id, false)}>Refuser</button>
                </div>
              </div>
            `)}
          </div>
        `}

        <h2 class="section-title" style="margin-top:26px">Historique retenu</h2>
        ${committed === null && html`<div class="skeleton" style="height:70px"></div>`}
        ${committed && committed.length === 0 && html`
          <div class="empty-state">
            <div class="glyph">📚</div>
            <div>Aucune écriture retenue pour l'instant.</div>
          </div>
        `}
        ${committed && committed.length > 0 && html`
          <div class="item-list">
            ${committed.slice(0, 60).map((it) => html`
              <div class="item-row" key=${it.id}>
                <div class="item-row-main">
                  <strong>${it.subject || '—'}</strong>
                  <span class="item-row-meta">${it.source || ''} ${it.predicate ? `· ${it.predicate}` : ''} ${it.object ? `→ ${it.object}` : ''}</span>
                </div>
              </div>
            `)}
          </div>
        `}
      </div>
    </div>
  `;
}
