import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

export function Webhooks() {
  const [data, setData] = useState(null);
  const [showForm, setShowForm] = useState(false);

  const load = useCallback(async () => {
    try { setData(await api.outboundWebhooks()); }
    catch (e) { toast(`Chargement impossible : ${e.message}`, 'err'); }
  }, []);

  useEffect(() => { load(); }, [load]);

  const remove = async (name) => {
    try {
      const res = await api.deleteWebhookEndpoint(name);
      toast(res.restart_required ? 'Supprimé — redémarrage du daemon requis pour appliquer' : 'Supprimé');
      await load();
    } catch (e) { toast(`Suppression impossible : ${e.message}`, 'err'); }
  };

  const test = async (endpoint) => {
    try {
      const res = await api.testWebhook({ url: endpoint.url, secret_env: endpoint.secret_env, dry_run: true });
      toast(`Test : ${res.status}`);
    } catch (e) { toast(`Test impossible : ${e.message}`, 'err'); }
  };

  const create = async (body) => {
    try {
      const res = await api.createWebhookEndpoint(body);
      setShowForm(false);
      toast(res.restart_required ? 'Créé — redémarrage du daemon requis pour appliquer' : 'Créé');
      await load();
    } catch (e) { toast(`Création impossible : ${e.message}`, 'err'); }
  };

  const endpoints = (data && data.endpoints) || [];

  // Rendue dans le hub Automation (onglet « Webhooks ») : le hub fournit
  // le wrapper .page et le titre, cette vue ne rend que son contenu.
  return html`
    <div>
        <p class="page-sub">Notifie des services externes sur les événements de Captain.</p>

        ${data && html`
          <div class="metrics-row">
            <div class="metric-chip ${data.enabled ? 'ok' : 'off'}">${data.enabled ? 'actifs' : 'désactivés'}</div>
            <div class="metric-chip">${endpoints.length} endpoint(s)</div>
            <div class="metric-chip">timeout ${data.timeout_secs}s · ${data.max_attempts} tentative(s)</div>
          </div>
          ${data.restart_required_for_config_changes && html`
            <p class="page-sub" style="color:var(--warn)">Toute création/modification/suppression nécessite un redémarrage du daemon pour prendre effet.</p>
          `}
        `}

        <div class="task-toolbar">
          <span class="spacer"></span>
          <button class="primary" onClick=${() => setShowForm((s) => !s)}>+ Endpoint</button>
        </div>
        ${showForm && html`<${WebhookForm} onCreate=${create} onCancel=${() => setShowForm(false)} />`}

        ${data === null && html`<div class="skeleton" style="height:70px"></div>`}
        ${data && endpoints.length === 0 && html`<div class="empty-state"><div class="glyph">🔗</div><div>Aucun endpoint configuré.</div></div>`}
        ${data && endpoints.length > 0 && html`
          <div class="item-list">
            ${endpoints.map((ep) => html`
              <div class="item-row" key=${ep.name}>
                <div class="item-row-main">
                  <strong>${ep.name}</strong>
                  <span class="item-row-meta">${ep.url}</span>
                  <span class="item-row-meta">${(ep.events || []).length ? (ep.events || []).join(', ') : 'tous les événements'}</span>
                </div>
                <div class="item-row-actions">
                  <span class="status-pill ${ep.enabled !== false ? 'status-done' : 'status-cancelled'}">${ep.enabled !== false ? 'actif' : 'inactif'}</span>
                  <button class="ghost" onClick=${() => test(ep)}>Tester</button>
                  <${DeleteButton} onConfirm=${() => remove(ep.name)} />
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

function WebhookForm({ onCreate, onCancel }) {
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [events, setEvents] = useState('');
  const [secretEnv, setSecretEnv] = useState('');

  const submit = (e) => {
    e.preventDefault();
    if (!name.trim() || !url.trim()) return;
    onCreate({
      name: name.trim(),
      url: url.trim(),
      events: events.split(',').map((s) => s.trim()).filter(Boolean),
      secret_env: secretEnv.trim() || null,
      enabled: true,
    });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <input type="text" placeholder="Nom" value=${name} onInput=${(e) => setName(e.target.value)} style="width:140px" />
      <input type="text" placeholder="URL" value=${url} onInput=${(e) => setUrl(e.target.value)} style="flex:1" />
      <input type="text" placeholder="Événements (séparés par ,)" value=${events}
        onInput=${(e) => setEvents(e.target.value)} style="width:200px" />
      <input type="text" placeholder="Variable d'env. du secret" value=${secretEnv}
        onInput=${(e) => setSecretEnv(e.target.value)} style="width:170px" />
      <button class="primary" type="submit">Créer</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}
