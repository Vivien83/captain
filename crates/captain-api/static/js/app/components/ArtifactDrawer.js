import { h } from '/assets/app/vendor/preact.module.js';
import { useEffect, useRef, useState } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api } from '../api.js';

const html = htm.bind(h);
const REFRESH_MS = 15000;

export function ArtifactDrawer({ open, onClose, onCount }) {
  const [items, setItems] = useState([]);
  const [status, setStatus] = useState(null);
  const [selectedId, setSelectedId] = useState(null);
  const [versions, setVersions] = useState([]);
  const [selectedVersion, setSelectedVersion] = useState(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [manualRefresh, setManualRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [error, setError] = useState('');
  const closeButton = useRef(null);

  useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    requestAnimationFrame(() => closeButton.current && closeButton.current.focus());
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  useEffect(() => {
    if (!open) return undefined;
    let cancelled = false;
    let inFlight = false;
    const load = async (quiet = false) => {
      if (inFlight) return;
      inFlight = true;
      if (!quiet) setLoading(true);
      try {
        const response = await api.artifacts(100);
        if (cancelled) return;
        const nextItems = Array.isArray(response.items) ? response.items : [];
        setItems(nextItems);
        setStatus(response.status || null);
        setError('');
        setSelectedId((current) => (
          nextItems.some((item) => item.artifact_id === current)
            ? current
            : ((nextItems[0] || {}).artifact_id || null)
        ));
        setRefreshTick((value) => value + 1);
        if (onCount) onCount(nextItems.length);
      } catch (loadError) {
        if (!cancelled) setError(loadError.message || 'Inventaire indisponible');
      } finally {
        inFlight = false;
        if (!cancelled) setLoading(false);
      }
    };
    load();
    const timer = setInterval(() => load(true), REFRESH_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [open, onCount, manualRefresh]);

  useEffect(() => {
    if (!open || !selectedId) {
      setVersions([]);
      setSelectedVersion(null);
      return undefined;
    }
    let cancelled = false;
    setVersionsLoading(true);
    api.artifactVersions(selectedId)
      .then((response) => {
        if (cancelled) return;
        const nextVersions = Array.isArray(response.items) ? response.items : [];
        setVersions(nextVersions);
        setSelectedVersion((current) => (
          nextVersions.some((item) => item.version === current)
            ? current
            : ((nextVersions[0] || {}).version || null)
        ));
      })
      .catch((loadError) => {
        if (!cancelled) {
          setVersions([]);
          setSelectedVersion(null);
          setError(loadError.message || 'Versions indisponibles');
        }
      })
      .finally(() => {
        if (!cancelled) setVersionsLoading(false);
      });
    return () => { cancelled = true; };
  }, [open, selectedId, refreshTick]);

  if (!open) return null;

  const selected = versions.find((item) => item.version === selectedVersion)
    || items.find((item) => item.artifact_id === selectedId)
    || null;
  const previewAvailable = selected && selected.preview_kind !== 'none';
  const previewUrl = selected
    ? api.artifactPreviewUrl(selected.artifact_id, selected.version)
    : '';
  const downloadUrl = selected
    ? api.artifactDownloadUrl(selected.artifact_id, selected.version)
    : '';

  return html`
    <div class="artifact-layer">
      <div class="artifact-scrim" onClick=${onClose}></div>
      <aside class="artifact-drawer" role="dialog" aria-modal="true" aria-labelledby="artifact-title">
        <header class="artifact-header">
          <div>
            <span class="artifact-eyebrow">Fichiers</span>
            <h2 id="artifact-title">Productions</h2>
          </div>
          <div class="artifact-header-actions">
            <button class="ghost artifact-icon-button" title="Actualiser" aria-label="Actualiser les fichiers"
              onClick=${() => setManualRefresh((value) => value + 1)}>↻</button>
            <button ref=${closeButton} class="ghost artifact-icon-button" title="Fermer" aria-label="Fermer les fichiers"
              onClick=${onClose}>×</button>
          </div>
        </header>

        <div class="artifact-summary ${status && !status.healthy ? 'warn' : ''}">
          <span class="status-dot ${status && !status.healthy ? 'warn' : ''}"></span>
          <strong>${status && !status.healthy ? 'Intégrité à vérifier' : 'Intégrité vérifiée'}</strong>
          <span>${status ? `${status.artifacts} fichier${status.artifacts === 1 ? '' : 's'} · ${status.versions} version${status.versions === 1 ? '' : 's'} · ${formatBytes(status.bytes)}` : 'Chargement'}</span>
        </div>

        ${error && html`<div class="artifact-error" role="alert">${error}</div>`}

        <div class="artifact-body">
          <section class="artifact-inventory" aria-label="Fichiers produits">
            ${loading && items.length === 0 && html`<div class="artifact-empty">Chargement…</div>`}
            ${!loading && items.length === 0 && !error && html`<div class="artifact-empty">Aucun fichier produit.</div>`}
            ${items.map((item) => html`
              <button key=${item.artifact_id}
                class="artifact-row ${item.artifact_id === selectedId ? 'active' : ''}"
                onClick=${() => setSelectedId(item.artifact_id)}>
                <span class="artifact-file-icon" aria-hidden="true">▤</span>
                <span class="artifact-row-copy">
                  <strong>${item.title}</strong>
                  <span>${item.filename}</span>
                  <small>${formatBytes(item.size_bytes)} · ${formatDate(item.created_at)}</small>
                </span>
                <span class="artifact-version-badge">v${item.version}</span>
              </button>
            `)}
          </section>

          <section class="artifact-preview" aria-label="Aperçu du fichier">
            ${selected ? html`
              <div class="artifact-detail-bar">
                <div class="artifact-detail-copy">
                  <strong>${selected.title}</strong>
                  <span>${selected.filename} · ${formatBytes(selected.size_bytes)}</span>
                </div>
                <select aria-label="Version du fichier" value=${selected.version}
                  disabled=${versionsLoading || versions.length < 2}
                  onChange=${(event) => setSelectedVersion(Number(event.target.value))}>
                  ${versions.map((version) => html`
                    <option value=${version.version}>v${version.version} · ${formatDate(version.created_at)}</option>
                  `)}
                </select>
                <a class="artifact-icon-action" href=${downloadUrl} download
                  title="Télécharger" aria-label="Télécharger ${selected.filename}">↓</a>
              </div>
              ${versionsLoading ? html`<div class="artifact-preview-empty">Chargement…</div>`
                : previewAvailable
                  ? html`<iframe key=${previewUrl} src=${previewUrl} sandbox="" referrerPolicy="no-referrer"
                      title=${`Aperçu de ${selected.filename}`}></iframe>`
                  : html`<div class="artifact-preview-empty">Aperçu indisponible pour ce format.</div>`}
            ` : html`<div class="artifact-preview-empty">Sélectionnez un fichier.</div>`}
          </section>
        </div>
      </aside>
    </div>
  `;
}

export function formatBytes(value) {
  const bytes = Number(value);
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 o';
  if (bytes < 1024) return `${Math.round(bytes)} o`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10240 ? 1 : 0)} Ko`;
  return `${(bytes / (1024 * 1024)).toFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} Mo`;
}

function formatDate(value) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return 'date inconnue';
  return new Intl.DateTimeFormat('fr-FR', {
    dateStyle: 'short',
    timeStyle: 'short',
  }).format(date);
}
