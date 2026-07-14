import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';
import { formatDuration, formatLatency, stateTone, statusSnapshot } from '../status_model.mjs';

const html = htm.bind(h);

export function Status() {
  const [snapshot, setSnapshot] = useState(null);
  const [showRaw, setShowRaw] = useState(false);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async (manual = false) => {
    if (manual) setRefreshing(true);
    try {
      setSnapshot(statusSnapshot(await api.status()));
    } catch (e) {
      toast(`Statut indisponible : ${e.message}`, 'err');
    } finally {
      if (manual) setRefreshing(false);
    }
  }, []);

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
              <${StatusMetric} label="Tool runs" value=${snapshot.toolRuns.running} meta=${snapshot.toolRuns.completed + ' terminés · ' + snapshot.toolRuns.failed + ' échecs · ' + snapshot.toolRuns.interrupted + ' interrompus'}
                tone=${snapshot.toolRuns.failed + snapshot.toolRuns.interrupted > 0 ? 'warn' : (snapshot.toolRuns.running > 0 ? 'ok' : 'neutral')} />
              <${StatusMetric} label="Streaming" value=${snapshot.streaming.active} meta=${snapshot.streaming.completed + ' flux terminés'} tone=${snapshot.streaming.active > 0 ? 'ok' : 'neutral'} />
              <${StatusMetric} label="Premier signal" value=${formatLatency(snapshot.streaming.firstSignalMs)} meta=${'premier token ' + formatLatency(snapshot.streaming.firstTokenMs)} />
              <${StatusMetric} label="Temps total" value=${formatLatency(snapshot.streaming.totalMs)} meta="dernier flux" />
              <${StatusMetric} label="Agent API" value=${snapshot.agentApi.state} meta=${snapshot.agentApi.pending + ' pending · ' + snapshot.agentApi.due + ' due · ' + snapshot.agentApi.deadLetters + ' dead letters'}
                tone=${snapshot.agentApi.due + snapshot.agentApi.deadLetters > 0 ? 'warn' : stateTone(snapshot.agentApi.state)} />
              <${StatusMetric} label="Budget" value=${formatNumber(snapshot.budget.totalTokens)} meta=${snapshot.budget.limitedAgents + ' agent(s) limité(s)'}
                tone=${snapshot.budget.actions.length > 0 ? 'warn' : 'neutral'} />
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

function StatusSection({ title, children }) {
  return html`
    <section class="status-section">
      <div class="status-section-head"><h2>${title}</h2></div>
      ${children}
    </section>
  `;
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
