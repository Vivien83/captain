import { h } from '/assets/app/vendor/preact.module.js';
import { useEffect, useRef, useState } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api } from '../api.js';
import { toast } from '../store.js';
import { formatBytes } from './ArtifactDrawer.js';

const html = htm.bind(h);
const REFRESH_MS = 3000;
const FILTERS = [
  ['all', 'Toutes'],
  ['running', 'En cours'],
  ['failed', 'Échecs'],
  ['interrupted', 'Interrompues'],
  ['cancelled', 'Annulées'],
];

export function LiveRunsDrawer({ open, onClose, onRunningCount }) {
  const [runs, setRuns] = useState([]);
  const [filter, setFilter] = useState('all');
  const [selectedId, setSelectedId] = useState(null);
  const [tail, setTail] = useState(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [manualRefresh, setManualRefresh] = useState(0);
  const [loading, setLoading] = useState(false);
  const [tailLoading, setTailLoading] = useState(false);
  const [cancelling, setCancelling] = useState(false);
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
        const response = await api.toolRuns(null, 200);
        if (cancelled) return;
        const nextRuns = Array.isArray(response.items) ? response.items : [];
        const visible = filterRuns(nextRuns, filter);
        setRuns(nextRuns);
        setError('');
        setSelectedId((current) => (
          visible.some((run) => run.run_id === current)
            ? current
            : ((visible[0] || {}).run_id || null)
        ));
        setRefreshTick((value) => value + 1);
        if (onRunningCount) {
          onRunningCount(nextRuns.filter((run) => run.status === 'running').length);
        }
      } catch (loadError) {
        if (!cancelled) setError(loadError.message || 'Exécutions indisponibles');
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
  }, [open, filter, manualRefresh, onRunningCount]);

  useEffect(() => {
    if (!open || !selectedId) {
      setTail(null);
      return undefined;
    }
    let cancelled = false;
    setTail((current) => (current && current.run_id === selectedId ? current : null));
    setTailLoading(true);
    api.toolRunTail(selectedId, 200)
      .then((response) => {
        if (!cancelled) {
          setTail(response.tail || null);
          setError('');
        }
      })
      .catch((loadError) => {
        if (!cancelled) {
          setTail(null);
          setError(loadError.message || 'Sortie indisponible');
        }
      })
      .finally(() => {
        if (!cancelled) setTailLoading(false);
      });
    return () => { cancelled = true; };
  }, [open, selectedId, refreshTick]);

  if (!open) return null;

  const visibleRuns = filterRuns(runs, filter);
  const selected = visibleRuns.find((run) => run.run_id === selectedId) || null;
  const counts = countStatuses(runs);

  const chooseFilter = (nextFilter) => {
    setFilter(nextFilter);
    setSelectedId(null);
  };

  const cancelSelected = async () => {
    if (!selected || !selected.cancellable || cancelling) return;
    if (!window.confirm(`Interrompre ${selected.tool_name} ?`)) return;
    setCancelling(true);
    try {
      await api.cancelToolRun(selected.run_id);
      toast('Exécution interrompue');
      setManualRefresh((value) => value + 1);
    } catch (cancelError) {
      setError(cancelError.message || 'Interruption impossible');
    } finally {
      setCancelling(false);
    }
  };

  return html`
    <div class="live-runs-layer">
      <div class="live-runs-scrim" onClick=${onClose}></div>
      <aside class="live-runs-drawer" role="dialog" aria-modal="true" aria-labelledby="live-runs-title">
        <header class="artifact-header">
          <div>
            <span class="artifact-eyebrow">Live Runs</span>
            <h2 id="live-runs-title">Exécutions</h2>
          </div>
          <div class="artifact-header-actions">
            <button class="ghost artifact-icon-button" title="Actualiser" aria-label="Actualiser les exécutions"
              onClick=${() => setManualRefresh((value) => value + 1)}>↻</button>
            <button ref=${closeButton} class="ghost artifact-icon-button" title="Fermer" aria-label="Fermer les exécutions"
              onClick=${onClose}>×</button>
          </div>
        </header>

        <div class="live-runs-summary">
          <span class="status-dot ${counts.failed + counts.interrupted > 0 ? 'warn' : ''}"></span>
          <strong>${counts.running} en cours</strong>
          <span>${counts.completed} terminée${counts.completed === 1 ? '' : 's'} · ${counts.failed} échec${counts.failed === 1 ? '' : 's'} · ${counts.interrupted} interrompue${counts.interrupted === 1 ? '' : 's'} · ${counts.cancelled} annulée${counts.cancelled === 1 ? '' : 's'}</span>
        </div>

        <div class="live-runs-filters" role="group" aria-label="Filtrer les exécutions">
          ${FILTERS.map(([value, label]) => html`
            <button key=${value} class=${filter === value ? 'active' : ''}
              aria-pressed=${filter === value} onClick=${() => chooseFilter(value)}>${label}</button>
          `)}
        </div>

        ${error && html`<div class="artifact-error" role="alert">${error}</div>`}

        <div class="live-runs-body">
          <section class="live-runs-list" aria-label="Liste des exécutions">
            ${loading && visibleRuns.length === 0 && html`<div class="artifact-empty">Chargement…</div>`}
            ${!loading && visibleRuns.length === 0 && !error && html`<div class="artifact-empty">Aucune exécution dans ce filtre.</div>`}
            ${visibleRuns.map((run) => html`
              <button key=${run.run_id} class="live-run-row ${run.run_id === selectedId ? 'active' : ''}"
                onClick=${() => setSelectedId(run.run_id)}>
                <span class="live-run-state ${run.status}" aria-hidden="true"></span>
                <span class="live-run-row-copy">
                  <strong>${run.tool_name}</strong>
                  <span>${statusLabel(run.status)} · ${formatElapsed(run.elapsed_ms)}</span>
                  <small>${shortRunId(run.run_id)} · ${formatUnix(run.started_at_unix_ms)}</small>
                </span>
                ${run.cancellable && html`<span class="live-run-live-badge">live</span>`}
              </button>
            `)}
          </section>

          <section class="live-run-detail" aria-label="Détail de l’exécution">
            ${selected ? html`
              <div class="live-run-detail-header">
                <div>
                  <span class="live-run-status ${selected.status}">${statusLabel(selected.status)}</span>
                  <h3>${selected.tool_name}</h3>
                  <code title=${selected.run_id}>${selected.run_id}</code>
                </div>
                ${selected.status === 'running' && (selected.cancellable
                  ? html`<button class="danger live-run-cancel" disabled=${cancelling}
                      onClick=${cancelSelected}>■ ${cancelling ? 'Arrêt…' : 'Arrêter'}</button>`
                  : html`<span class="live-run-noncancellable">Non annulable</span>`)}
              </div>

              <div class="live-run-metadata">
                <${Metadata} label="Démarrage" value=${formatUnix(selected.started_at_unix_ms)} />
                <${Metadata} label="Durée" value=${formatElapsed(selected.elapsed_ms)} />
                <${Metadata} label="Agent" value=${selected.caller_agent_id || 'runtime'} />
                <${Metadata} label="Mode" value=${selected.detached ? 'détaché' : 'foreground'} />
                <${Metadata} label="Sortie" value=${selected.output_available ? formatOutput(selected) : 'aucune preuve'} />
                <${Metadata} label="Retry" value=${selected.retry_attempt > 0 ? `tentative ${selected.retry_attempt}` : 'origine'} />
              </div>

              ${selected.input_sha256 && html`
                <div class="live-run-digest">
                  <span>Digest input</span>
                  <code>${selected.input_sha256}</code>
                </div>
              `}

              <div class="live-run-tail-head">
                <strong>Fin de sortie</strong>
                <span>${tail ? `lignes ${tail.start_line}–${tail.end_line} / ${tail.total_lines}` : 'chargement'}</span>
              </div>
              <div class="live-run-tail ${tail && tail.content_withheld ? 'withheld' : ''}">
                ${tailLoading && !tail ? html`<span>Chargement…</span>`
                  : tail && tail.content
                    ? html`<pre>${tail.content}</pre>`
                    : html`<span>Aucune sortie retenue.</span>`}
              </div>
              ${tail && (tail.content_truncated || tail.content_withheld) && html`
                <div class="live-run-tail-note">
                  ${tail.content_withheld ? 'Contenu masqué par le garde secrets.' : 'Tail limité par la fenêtre opérateur.'}
                </div>
              `}
            ` : html`<div class="artifact-preview-empty">Sélectionnez une exécution.</div>`}
          </section>
        </div>
      </aside>
    </div>
  `;
}

function Metadata({ label, value }) {
  return html`<div><span>${label}</span><strong>${value}</strong></div>`;
}

function filterRuns(runs, filter) {
  return filter === 'all' ? runs : runs.filter((run) => run.status === filter);
}

function countStatuses(runs) {
  const counts = { running: 0, completed: 0, failed: 0, interrupted: 0, cancelled: 0 };
  runs.forEach((run) => {
    if (Object.hasOwn(counts, run.status)) counts[run.status] += 1;
  });
  return counts;
}

function statusLabel(status) {
  return ({
    running: 'En cours',
    completed: 'Terminée',
    failed: 'Échec',
    cancelled: 'Annulée',
    interrupted: 'Interrompue',
  })[status] || status;
}

function shortRunId(runId) {
  return runId.length > 22 ? `${runId.slice(0, 14)}…${runId.slice(-6)}` : runId;
}

function formatElapsed(value) {
  const milliseconds = Number(value);
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return 'durée inconnue';
  if (milliseconds < 1000) return `${Math.round(milliseconds)} ms`;
  if (milliseconds < 60000) return `${(milliseconds / 1000).toFixed(milliseconds < 10000 ? 1 : 0)} s`;
  const minutes = Math.floor(milliseconds / 60000);
  const seconds = Math.floor((milliseconds % 60000) / 1000);
  return `${minutes} min ${seconds.toString().padStart(2, '0')} s`;
}

function formatUnix(value) {
  const date = new Date(Number(value));
  if (Number.isNaN(date.getTime())) return 'date inconnue';
  return new Intl.DateTimeFormat('fr-FR', {
    dateStyle: 'short',
    timeStyle: 'medium',
  }).format(date);
}

function formatOutput(run) {
  const observed = run.output_total_bytes ?? run.output_stored_bytes;
  const flags = [];
  if (run.output_redacted) flags.push('expurgée');
  if (run.output_capped) flags.push('plafonnée');
  return `${formatBytes(observed || 0)}${flags.length ? ` · ${flags.join(', ')}` : ''}`;
}
