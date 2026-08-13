import { h } from '/assets/app/vendor/preact.module.js';
import { useState, useEffect, useCallback } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api } from '../api.js';
import { getState, toast } from '../store.js';
import { formatDuration, formatLatency, stateTone, statusSnapshot } from '../status_model.mjs';

const html = htm.bind(h);

export function Status() {
  const [snapshot, setSnapshot] = useState(null);
  const [deviceRegistry, setDeviceRegistry] = useState({
    available: true,
    devices: [],
    requests: [],
    enrollment: null,
  });
  const [showRaw, setShowRaw] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [deviceBusy, setDeviceBusy] = useState(false);
  const clientMode = getState().clientMode === true;

  const loadDevices = useCallback(async () => {
    if (clientMode) {
      setDeviceRegistry({ available: false, devices: [], requests: [], enrollment: null });
      return;
    }
    try {
      const [devices, requests, enrollment] = await Promise.all([
        api.hubDevices(),
        api.hubPairingRequests(),
        api.hubPairingEnrollment(),
      ]);
      setDeviceRegistry({
        available: true,
        devices: devices.devices || [],
        requests: requests.requests || [],
        enrollment,
      });
    } catch {
      setDeviceRegistry((current) => ({ ...current, available: false }));
    }
  }, [clientMode]);

  const load = useCallback(async (manual = false) => {
    if (manual) setRefreshing(true);
    try {
      setSnapshot(statusSnapshot(await api.status()));
      await loadDevices();
    } catch (e) {
      toast(`Statut indisponible : ${e.message}`, 'err');
    } finally {
      if (manual) setRefreshing(false);
    }
  }, [loadDevices]);

  const toggleEnrollment = async () => {
    setDeviceBusy(true);
    try {
      if (deviceRegistry.enrollment && deviceRegistry.enrollment.open) {
        await api.closeHubPairingEnrollment();
        toast('Ajout d’appareil fermé');
      } else {
        await api.openHubPairingEnrollment(600);
        toast('Ajout d’appareil ouvert pendant 10 minutes');
      }
      await loadDevices();
    } catch (error) {
      toast(`Action impossible : ${error.message}`, 'err');
    } finally {
      setDeviceBusy(false);
    }
  };

  const denyPairing = async (requestId) => {
    setDeviceBusy(true);
    try {
      await api.denyHubPairingRequest(requestId);
      toast('Demande d’appairage refusée');
      await loadDevices();
    } catch (error) {
      toast(`Refus impossible : ${error.message}`, 'err');
    } finally {
      setDeviceBusy(false);
    }
  };

  useEffect(() => {
    load();
    const timer = setInterval(load, 5000);
    return () => clearInterval(timer);
  }, [load]);

  return html`
    <div class="page">
      <div class="page-inner page-inner-wide status-page">
        <div class="page-heading">
          <div>
            <h1 class="page-title">Status</h1>
            <p class="page-sub">État opérationnel du runtime et actions qui demandent ton attention.</p>
          </div>
          <button class="ghost" disabled=${refreshing} onClick=${() => load(true)}>${refreshing ? 'Actualisation…' : 'Actualiser'}</button>
        </div>

        ${snapshot === null && html`<div class="skeleton" style="height:150px"></div>`}
        ${snapshot && html`
          <section class="status-banner tone-${stateTone(snapshot.health.state)}">
            <div class="status-banner-main">
              <span class="status-health-dot"></span>
              <div>
                <span class="status-label">Runtime health</span>
                <strong>${snapshot.health.state}</strong>
              </div>
            </div>
            <div class="status-banner-meta">
              <span>${snapshot.version}</span>
              <span>${snapshot.provider} / ${snapshot.model}</span>
              <span>uptime ${formatDuration(snapshot.uptimeSeconds)}</span>
            </div>
          </section>

          ${snapshot.health.issues.length > 0 && html`
            <section class="status-section status-attention">
              <div class="status-section-head">
                <h2>Attention</h2>
                <span>${snapshot.health.issueCount} signal(s)</span>
              </div>
              <div class="status-issue-list">
                ${snapshot.health.issues.map((issue) => html`
                  <div class="status-issue tone-${stateTone(issue.severity)}" key=${issue.kind}>
                    <div>
                      <strong>${issue.summary || issue.kind}</strong>
                      ${issue.action && html`<span>${issue.action}</span>`}
                    </div>
                    <span class="status-pill status-${issue.severity === 'critical' ? 'blocked' : 'review'}">${issue.severity}</span>
                  </div>
                `)}
              </div>
            </section>
          `}

          <${StatusSection} title="Runtime">
            <div class="status-grid">
              <${StatusMetric} label="Agents" value=${snapshot.agents} meta=${snapshot.activeRuns + ' run(s) actif(s)'} tone=${snapshot.activeRuns > 0 ? 'ok' : 'neutral'} />
              <${StatusMetric} label="Processus" value=${snapshot.processes} meta="suivis par Captain" tone=${snapshot.processes > 0 ? 'ok' : 'neutral'} />
              <${StatusMetric} label="Disque libre" value=${snapshot.disk.availableGiB === null ? '—' : snapshot.disk.availableGiB.toFixed(1) + ' GiB'}
                meta=${snapshot.disk.cleanupRecommended ? 'nettoyage recommandé' : 'au-dessus du seuil'} tone=${snapshot.disk.cleanupRecommended ? 'warn' : 'ok'} />
              <${StatusMetric} label="Shutdown" value=${snapshot.shutdown.status} meta=${snapshot.shutdown.activeWork + ' travail(aux) actif(s)'}
                tone=${stateTone(snapshot.shutdown.status)} />
              <${StatusMetric} label="LLM driver" value=${snapshot.llmReady ? 'ready' : 'unavailable'} meta=${snapshot.provider + ' / ' + snapshot.model}
                tone=${snapshot.llmReady ? 'ok' : 'err'} />
              <${StatusMetric} label="Accès" value=${snapshot.access.authMode} meta=${snapshot.access.networkEnabled ? 'réseau activé' : 'réseau désactivé'} />
            </div>
          <//>

          <${StatusSection} title="Execution">
            <div class="status-grid">
              <${StatusMetric} label="Profil" value=${snapshot.execution.profile}
                meta=${'hôte ' + (snapshot.execution.hostAllowed ? 'autorisé' : 'bloqué') + ' · ' + snapshot.execution.backend}
                tone=${snapshot.execution.hostAllowed ? 'neutral' : 'ok'} />
              <${StatusMetric} label="Politique hôte" value=${snapshot.execution.effectiveMode + ' / ' + snapshot.execution.criticalMode}
                meta=${snapshot.execution.configuredMode === snapshot.execution.effectiveMode
                  ? snapshot.execution.isolation + ' · sans isolation OS'
                  : 'configuré ' + snapshot.execution.configuredMode + ' · ' + snapshot.execution.isolation}
                tone=${snapshot.execution.effectiveMode === 'full' ? 'warn' : 'ok'} />
              <${StatusMetric} label="Rail Docker" value=${snapshot.execution.docker.enabled ? 'activé' : 'désactivé'}
                meta=${snapshot.execution.routing + ' · runtime ' + snapshot.execution.docker.availability + ' · aucun repli hôte'}
                tone=${snapshot.execution.docker.violations.length > 0 ? 'warn' : (snapshot.execution.docker.enabled ? 'ok' : 'neutral')} />
              <${StatusMetric} label="Tool runs" value=${snapshot.toolRuns.running} meta=${snapshot.toolRuns.completed + ' terminés · ' + snapshot.toolRuns.failed + ' échecs · ' + snapshot.toolRuns.interrupted + ' interrompus'}
                tone=${snapshot.toolRuns.failed + snapshot.toolRuns.interrupted > 0 ? 'warn' : (snapshot.toolRuns.running > 0 ? 'ok' : 'neutral')} />
              <${StatusMetric} label="Streaming" value=${snapshot.streaming.active} meta=${snapshot.streaming.completed + ' flux terminés'} tone=${snapshot.streaming.active > 0 ? 'ok' : 'neutral'} />
              <${StatusMetric} label="Premier signal" value=${formatLatency(snapshot.streaming.firstSignalMs)} meta=${'premier token ' + formatLatency(snapshot.streaming.firstTokenMs)} />
              <${StatusMetric} label="Temps total" value=${formatLatency(snapshot.streaming.totalMs)} meta="dernier flux" />
              <${StatusMetric} label="Agent API" value=${snapshot.agentApi.state} meta=${snapshot.agentApi.pending + ' pending · ' + snapshot.agentApi.due + ' due · ' + snapshot.agentApi.deadLetters + ' dead letters'}
                tone=${snapshot.agentApi.due + snapshot.agentApi.deadLetters > 0 ? 'warn' : stateTone(snapshot.agentApi.state)} />
            </div>
          <//>

          <${StatusSection} title="Déploiement">
            <div class="status-grid">
              <${StatusMetric} label="Readiness" value=${snapshot.deployment.readiness.state}
                meta=${snapshot.deployment.readiness.checkedAt
                  ? 'vérifié ' + formatTimestamp(snapshot.deployment.readiness.checkedAt) + ' · prochain ' + formatTimestamp(snapshot.deployment.readiness.nextCheckAt)
                  : snapshot.deployment.readiness.state === 'not_configured'
                    ? 'aucun domaine public configuré'
                    : snapshot.deployment.readiness.checks.length + ' contrôle(s) · premier passage en attente'}
                tone=${stateTone(snapshot.deployment.readiness.state)} />
              <${StatusMetric} label="Domaine public" value=${snapshot.deployment.publicUrl || 'non configuré'}
                meta=${snapshot.deployment.publicUrl
                  ? snapshot.deployment.profile + ' · ' + (snapshot.deployment.https ? 'HTTPS' : 'HTTP')
                  : 'profil ' + snapshot.deployment.profile}
                tone=${snapshot.deployment.publicUrl && !snapshot.deployment.https ? 'warn' : 'neutral'} />
              <${StatusMetric} label="Reverse proxy" value=${snapshot.deployment.reverseProxy || 'non configuré'}
                meta=${snapshot.deployment.readiness.durationMs === null
                  ? 'premier contrôle en attente'
                  : 'sonde ' + formatLatency(snapshot.deployment.readiness.durationMs)} />
            </div>
            ${snapshot.deployment.readiness.checks.length > 0 && html`
              <div class="status-issue-list">
                ${snapshot.deployment.readiness.checks.map((check) => html`
                  <div class="status-issue tone-${stateTone(check.status)}" key=${check.id}>
                    <div>
                      <strong>${check.summary}</strong>
                    </div>
                    <span class="status-pill status-${check.status === 'ok' || check.status === 'skipped' ? 'done' : (check.status === 'failed' ? 'blocked' : 'review')}">${check.status}</span>
                  </div>
                `)}
              </div>
            `}
            ${snapshot.deployment.readiness.actions.length > 0 && html`
              <${TextSignals} items=${snapshot.deployment.readiness.actions} actions=${true} />
            `}
          <//>

          ${!clientMode && html`<${StatusSection} title="Appareils" actions=${deviceRegistry.available && html`
            <button class=${deviceRegistry.enrollment && deviceRegistry.enrollment.open ? 'ghost' : 'primary'}
              disabled=${deviceBusy} onClick=${toggleEnrollment}>
              ${deviceRegistry.enrollment && deviceRegistry.enrollment.open ? 'Fermer l’ajout' : 'Ajouter un appareil'}
            </button>
          `}>
            ${!deviceRegistry.available && html`
              <div class="status-empty">Le registre d’appareils est désactivé ou momentanément indisponible.</div>
            `}
            ${deviceRegistry.available && html`
              ${deviceRegistry.enrollment && deviceRegistry.enrollment.open && html`
                <div class="device-enrollment-notice">
                  <span class="status-dot"></span>
                  Ajout ouvert jusqu’à ${formatTimestamp(deviceRegistry.enrollment.expires_at_ms)}
                </div>
              `}
              <${PairingCodeReview} enrollment=${deviceRegistry.enrollment}
                busy=${deviceBusy} onBusy=${setDeviceBusy} onRefresh=${loadDevices} />
              ${deviceRegistry.requests.length > 0 && html`
                <div class="device-pending-list">
                  ${deviceRegistry.requests.map((request) => html`
                    <div class="device-pending-row" key=${request.request_id}>
                      <div>
                        <strong>${request.display_name}</strong>
                        <span>${request.role} · ${request.platform} · ${request.captain_version}</span>
                      </div>
                      <button class="ghost danger" disabled=${deviceBusy}
                        onClick=${() => denyPairing(request.request_id)}>Refuser</button>
                    </div>
                  `)}
                </div>
              `}
              <div class="device-list">
                ${deviceRegistry.devices.map((device) => html`
                  <${DeviceRow} key=${device.device_id} device=${device}
                    busy=${deviceBusy} onBusy=${setDeviceBusy} onRefresh=${loadDevices} />
                `)}
                ${deviceRegistry.devices.length === 0 && html`
                  <div class="status-empty">Aucun Client ou Nœud appairé.</div>
                `}
              </div>
            `}
          <//>`}

          <${StatusSection} title="Quotas">
            <div class="status-grid">
              <${StatusMetric} label="Captain internal" value=${formatNumber(snapshot.budget.totalTokens)} meta=${snapshot.budget.limitedAgents + ' agent(s) · fenêtre glissante locale'}
                tone=${snapshot.budget.actions.some((item) => !item.startsWith('Provider subscription')) ? 'warn' : 'neutral'} />
              ${snapshot.budget.provider.items.length === 0 && html`
                <${StatusMetric} label="Provider subscription" value="non observé" meta="signaux officiels provider uniquement"
                  tone="neutral" />
              `}
              ${snapshot.budget.provider.items.map((quota) => html`
                <${StatusMetric} key=${quota.provider + ':' + quota.id} label=${quota.name}
                  value=${providerQuotaWindows(quota)}
                  meta=${providerQuotaMeta(quota)} tone=${quotaTone(quota)} />
              `)}
            </div>
          <//>

          <${StatusSection} title="Workload">
            <div class="status-grid">
              <${StatusMetric} label="Projets actifs" value=${snapshot.workload.projectsActive} meta=${snapshot.workload.projectAttention + ' demandent attention'}
                tone=${snapshot.workload.projectAttention > 0 ? 'warn' : 'ok'} />
              <${StatusMetric} label="Goals" value=${snapshot.workload.goalsActive} meta=${snapshot.workload.goalsEscalated + ' escaladé(s)'}
                tone=${snapshot.workload.goalsEscalated > 0 ? 'warn' : 'neutral'} />
              <${StatusMetric} label="Crons" value=${snapshot.workload.cronEnabled} meta=${snapshot.workload.cronDue + ' dû(s)'}
                tone=${snapshot.workload.cronDue > 0 ? 'warn' : 'neutral'} />
              <${StatusMetric} label="Livraisons" value=${snapshot.workload.deliveryDue} meta=${snapshot.workload.deliveryDeadLetters + ' dead letter(s)'}
                tone=${snapshot.workload.deliveryDue + snapshot.workload.deliveryDeadLetters > 0 ? 'warn' : 'ok'} />
              <${StatusMetric} label="Channels" value=${snapshot.channels.ready + '/' + snapshot.channels.total} meta=${snapshot.channels.locked + ' verrouillé(s) · ' + snapshot.channels.pendingMessages + ' message(s) pending'}
                tone=${snapshot.channels.locked + snapshot.channels.deadLetters > 0 ? 'warn' : 'ok'} />
              <${StatusMetric} label="Channels ready" value=${snapshot.channels.readyNames.length ? snapshot.channels.readyNames.join(', ') : 'aucun'}
                meta=${snapshot.channels.configured + ' configuré(s)'} />
            </div>
          <//>

          <section class="status-section">
            <div class="status-columns">
              <div>
                <div class="status-section-head"><h2>Awareness</h2><span class="status-pill status-${snapshot.consciousness.state === 'steady' ? 'done' : 'review'}">${snapshot.consciousness.state}</span></div>
                <${TextSignals} items=${snapshot.consciousness.signals} empty="Aucun signal actif." />
                <${TextSignals} items=${snapshot.consciousness.actions} empty="Aucune action requise." actions=${true} />
              </div>
              <div>
                <div class="status-section-head"><h2>Native</h2></div>
                <div class="native-status-row"><span>Embeddings</span><${ReadyState} value=${snapshot.native.embeddings} /></div>
                <div class="native-status-row"><span>Speech to text</span><${ReadyState} value=${snapshot.native.stt} /></div>
                <div class="native-status-row"><span>Text to speech</span><${ReadyState} value=${snapshot.native.tts} /></div>
              </div>
            </div>
          </section>

          ${snapshot.budget.actions.length > 0 && html`
            <section class="status-section">
              <div class="status-section-head"><h2>Budget actions</h2></div>
              <${TextSignals} items=${snapshot.budget.actions} actions=${true} />
            </section>
          `}

          <div class="status-raw-toggle">
            <button class="ghost" onClick=${() => setShowRaw((value) => !value)}>${showRaw ? 'Masquer' : 'Afficher'} le contrat brut</button>
            ${showRaw && html`<pre class="code-block">${JSON.stringify(snapshot.raw, null, 2)}</pre>`}
          </div>
        `}
      </div>
    </div>
  `;
}

function StatusSection({ title, actions = null, children }) {
  return html`
    <section class="status-section">
      <div class="status-section-head"><h2>${title}</h2>${actions}</div>
      ${children}
    </section>
  `;
}

function PairingCodeReview({ enrollment, busy, onBusy, onRefresh }) {
  const [code, setCode] = useState('');
  const [review, setReview] = useState(null);
  const [allowMutation, setAllowMutation] = useState(false);

  const inspect = async () => {
    const normalized = code.trim().toUpperCase();
    if (!normalized) return;
    onBusy(true);
    try {
      const next = await api.reviewHubPairingCode(normalized);
      setCode(normalized);
      setReview(next);
      setAllowMutation(false);
    } catch (error) {
      setReview(null);
      toast(`Code invalide ou expiré : ${error.message}`, 'err');
    } finally {
      onBusy(false);
    }
  };

  const approve = async () => {
    if (!review) return;
    onBusy(true);
    try {
      const grant = {
        ...review.requested_grants,
        allow_mutation: Boolean(review.requested_grants.allow_mutation && allowMutation),
      };
      await api.approveHubPairingCode(code, grant);
      toast('Appareil approuvé');
      setCode('');
      setReview(null);
      setAllowMutation(false);
      await onRefresh();
    } catch (error) {
      toast(`Approbation impossible : ${error.message}`, 'err');
    } finally {
      onBusy(false);
    }
  };

  if (!enrollment || !enrollment.open) return null;
  return html`
    <div class="device-code-review">
      <div class="device-code-input">
        <label for="pairing-display-code">Code affiché sur l’appareil</label>
        <div>
          <input id="pairing-display-code" value=${code} maxlength="16" autocomplete="off"
            placeholder="ABCD-EFGH" onInput=${(event) => setCode(event.target.value)} />
          <button disabled=${busy || !code.trim()} onClick=${inspect}>Examiner</button>
        </div>
      </div>
      ${review && html`
        <div class="device-review-detail">
          <div>
            <strong>${review.display_name}</strong>
            <span>${review.role} · ${review.platform} · ${review.captain_version}</span>
            <span>${grantSummary(review.requested_grants)}</span>
          </div>
          ${review.requested_grants.allow_mutation && html`
            <label class="device-mutation-toggle">
              <input type="checkbox" checked=${allowMutation}
                onChange=${(event) => setAllowMutation(event.target.checked)} />
              Autoriser les modifications locales
            </label>
          `}
          <button class="primary" disabled=${busy} onClick=${approve}>
            ${allowMutation ? 'Approuver avec écriture' : 'Approuver en lecture seule'}
          </button>
        </div>
      `}
    </div>
  `;
}

function DeviceRow({ device, busy, onBusy, onRefresh }) {
  const [confirmRevoke, setConfirmRevoke] = useState(false);
  const revoke = async () => {
    if (!confirmRevoke) {
      setConfirmRevoke(true);
      return;
    }
    onBusy(true);
    try {
      await api.revokeHubDevice(device.device_id);
      toast(`${device.display_name} révoqué`);
      await onRefresh();
    } catch (error) {
      toast(`Révocation impossible : ${error.message}`, 'err');
    } finally {
      onBusy(false);
      setConfirmRevoke(false);
    }
  };
  return html`
    <div class="device-row">
      <span class="device-state-dot state-${device.status}"></span>
      <div class="device-row-main">
        <div><strong>${device.display_name}</strong><span class="status-pill status-${device.status === 'online' ? 'done' : (device.status === 'revoked' ? 'blocked' : 'review')}">${device.status}</span></div>
        <span>${device.role} · ${device.platform} · ${device.captain_version}</span>
        <span>${capabilitySummary(device)}</span>
        ${device.presence && device.presence.action && html`<span class="device-actionable">${device.presence.action}</span>`}
      </div>
      ${device.registry_status !== 'revoked' && html`
        <button class="ghost danger" disabled=${busy} onClick=${revoke}>
          ${confirmRevoke ? 'Confirmer' : 'Révoquer'}
        </button>
      `}
    </div>
  `;
}

function grantSummary(grant = {}) {
  const workspaces = (grant.workspace_ids || []).length;
  const tools = (grant.tool_families || []).length;
  return `${workspaces} workspace(s) · ${tools} famille(s) d’outils · ${grant.allow_mutation ? 'écriture demandée' : 'lecture seule'}`;
}

function capabilitySummary(device) {
  if (device.role === 'client') return `Dernière présence ${formatTimestamp(device.last_seen_ms)}`;
  const workspaces = ((device.capabilities || {}).workspaces || []).map((workspace) => workspace.label).join(', ');
  const transport = device.last_transport || 'aucun transport';
  return `${workspaces || 'aucun workspace'} · ${transport} · dernière présence ${formatTimestamp(device.last_seen_ms)}`;
}

function StatusMetric({ label, value, meta, tone = 'neutral' }) {
  return html`
    <div class="status-cell tone-${tone}">
      <span class="status-label">${label}</span>
      <strong class="status-value">${value}</strong>
      <span class="status-meta">${meta}</span>
    </div>
  `;
}

function TextSignals({ items = [], empty = '', actions = false }) {
  if (!items.length) return empty ? html`<div class="status-empty">${empty}</div>` : null;
  return html`
    <ul class="status-signal-list ${actions ? 'actions' : ''}">
      ${items.map((item, index) => html`<li key=${index + ':' + item}>${item}</li>`)}
    </ul>
  `;
}

function ReadyState({ value }) {
  const label = value === null ? 'unknown' : (value ? 'ready' : 'not ready');
  const className = value === null ? 'status-review' : (value ? 'status-done' : 'status-blocked');
  return html`<span class="status-pill ${className}">${label}</span>`;
}

function formatNumber(value) {
  return Number(value || 0).toLocaleString('fr-FR');
}

function formatTimestamp(value) {
  if (!value) return 'jamais';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString('fr-FR', {
    day: '2-digit',
    month: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

function providerQuotaWindows(quota) {
  const windows = [quota.primary, quota.secondary]
    .filter(Boolean)
    .map((window) => {
      const duration = quotaDuration(window.windowSeconds);
      return `${duration} ${window.remainingPercent.toFixed(1)}% reste`;
    });
  if (quota.spendControl) {
    windows.push(
      `mensuel ${quota.spendControl.remainingPercent}% reste (${quota.spendControl.remaining}/${quota.spendControl.limit})`,
    );
  }
  return windows.length > 0 ? windows.join(' · ') : quota.alert;
}

function providerQuotaMeta(quota) {
  const parts = [quota.provider];
  if (quota.plan) parts.push(quota.plan);
  parts.push(quota.alert);
  if (quota.stale) parts.push('stale');
  const resets = [quota.primary, quota.secondary]
    .filter(Boolean)
    .filter((window) => window.resetsAt)
    .map((window) => `${quotaDuration(window.windowSeconds)} ${window.resetsAt}`);
  if (resets.length > 0) parts.push(`reset ${resets.join(' · ')}`);
  if (quota.spendControl && quota.spendControl.resetsAt) {
    parts.push(`reset mensuel ${quota.spendControl.resetsAt}`);
  }
  return parts.join(' · ');
}

function quotaDuration(seconds) {
  if (!seconds) return 'fenêtre';
  if (seconds % 604800 === 0) return `${seconds / 604800} sem.`;
  if (seconds % 86400 === 0) return `${seconds / 86400} j`;
  if (seconds % 3600 === 0) return `${seconds / 3600} h`;
  if (seconds % 60 === 0) return `${seconds / 60} min`;
  return `${seconds} s`;
}

function quotaTone(quota) {
  if (quota.alert === 'exhausted') return 'err';
  if (quota.alert === 'critical' || quota.alert === 'warning' || quota.stale) return 'warn';
  return 'ok';
}
