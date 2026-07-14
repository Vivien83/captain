import { h } from 'preact';
import { useState } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';

const html = htm.bind(h);

const STATUS_LABELS = {
  ready: 'Prêt', running: 'En cours', paused: 'En pause',
  blocked: 'Bloqué', failed: 'Échoué', done: 'Terminé',
};
const STATUS_PILL_CLASS = {
  ready: 'status-todo', running: 'status-doing', paused: 'status-review',
  blocked: 'status-blocked', failed: 'status-cancelled', done: 'status-done',
};
const PHASE_LABELS = {
  observe: 'Observer', think: 'Réfléchir', plan: 'Planifier', build: 'Construire',
  execute: 'Exécuter', verify: 'Vérifier', learn: 'Apprendre', unknown: '—',
};
const WORKER_STATUS_CLASS = {
  running: 'status-doing', done: 'status-done', failed: 'status-cancelled',
  blocked: 'status-blocked', paused: 'status-review',
};

// Runtime status/phase drive the summary strip, controls, pending asks/tool
// requests, worker list, and a bounded timeline — everything the old
// roadmap.js runtime dashboard exposed, ported so it isn't lost when that
// file goes away. `onRefresh` re-fetches after every mutating action; the
// caller (Projects.js) also polls this on an interval while a run is live.
export function ProjectRuntime({ projectId, runtime, operatorStatus, onRefresh }) {
  const [busy, setBusy] = useState(false);
  if (!runtime) return null;

  const act = async (fn, label) => {
    setBusy(true);
    try {
      await fn(projectId);
      toast(label);
      await onRefresh();
    } catch (e) {
      toast(`Action impossible : ${e.message}`, 'err');
    } finally {
      setBusy(false);
    }
  };

  const questions = (runtime.user_questions || []).filter((q) => q.status === 'pending');
  const toolRequest = operatorStatus && operatorStatus.pending_tool_request;

  return html`
    <div class="runtime-panel">
      <div class="metrics-row" style="margin-bottom:14px">
        <span class="status-pill ${STATUS_PILL_CLASS[runtime.status] || 'status-todo'}">${STATUS_LABELS[runtime.status] || runtime.status}</span>
        <div class="metric-chip">phase : ${PHASE_LABELS[runtime.current_phase] || runtime.current_phase}</div>
        <div class="metric-chip">${runtime.progress ?? 0}%</div>
        ${runtime.manager_agent && runtime.manager_agent.name && html`
          <div class="metric-chip">manager : ${runtime.manager_agent.name}${runtime.manager_agent.model ? ` (${runtime.manager_agent.model})` : ''}</div>
        `}
        ${runtime.parallelism && runtime.parallelism.max_parallel_agents != null && html`
          <div class="metric-chip">${runtime.parallelism.running || 0}/${runtime.parallelism.max_parallel_agents} worker(s)</div>
        `}
      </div>

      <div class="task-toolbar">
        ${runtime.status === 'running' ? html`
          <button class="ghost" disabled=${busy} onClick=${() => act(api.pauseProjectRuntime, 'Run mis en pause')}>Pause</button>
          <button class="ghost" disabled=${busy} onClick=${() => act(api.takeoverProjectRuntime, 'Contrôle repris')}>Reprendre la main</button>
        ` : html`
          ${(runtime.status === 'paused') && html`
            <button class="ghost" disabled=${busy} onClick=${() => act(api.resumeProjectRuntime, 'Run repris')}>Reprendre</button>
          `}
          <button class="primary" disabled=${busy} onClick=${() => act(api.startProjectRuntime, 'Run démarré')}>Démarrer le run</button>
        `}
      </div>

      ${questions.length > 0 && html`<${AskPanel} projectId=${projectId} question=${questions[0]} onDone=${onRefresh} />`}
      ${toolRequest && html`<${ToolRequestPanel} projectId=${projectId} request=${toolRequest} onDone=${onRefresh} />`}

      ${runtime.workers && runtime.workers.length > 0 && html`
        <h2 class="section-title" style="margin-top:20px">Workers</h2>
        <div class="item-list">
          ${runtime.workers.map((w) => html`
            <div class="item-row" key=${w.id}>
              <div class="item-row-main">
                <strong>${w.role || w.id}</strong>
                <span class="item-row-meta">${PHASE_LABELS[w.phase] || w.phase}${w.summary ? ` · ${w.summary}` : ''}</span>
              </div>
              <span class="status-pill ${WORKER_STATUS_CLASS[w.status] || 'status-todo'}">${w.status}</span>
            </div>
          `)}
        </div>
      `}

      ${runtime.timeline && runtime.timeline.length > 0 && html`
        <h2 class="section-title" style="margin-top:20px">Timeline</h2>
        <div class="item-list">
          ${runtime.timeline.slice().reverse().slice(0, 40).map((ev, i) => html`
            <div class="item-row" key=${ev.id || i}>
              <div class="item-row-main">
                <strong>${ev.title || ev.kind}</strong>
                ${ev.detail && html`<span class="item-row-meta">${ev.detail}</span>`}
                <span class="item-row-meta">${PHASE_LABELS[ev.phase] || ev.phase} · ${ev.actor || ''}</span>
              </div>
            </div>
          `)}
        </div>
      `}
    </div>
  `;
}

function AskPanel({ projectId, question, onDone }) {
  const [answer, setAnswer] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = async (value) => {
    if (!value.trim()) return;
    setBusy(true);
    try {
      await api.answerProjectAsk(projectId, { ask_id: question.ask_id, answer: value.trim() });
      setAnswer('');
      toast('Réponse envoyée');
      await onDone();
    } catch (e) {
      toast(`Envoi impossible : ${e.message}`, 'err');
    } finally {
      setBusy(false);
    }
  };

  return html`
    <div class="ask-panel" style="margin-top:14px">
      <div class="item-row-meta" style="margin-bottom:6px">Captain a une question (${PHASE_LABELS[question.phase] || question.phase})</div>
      <strong style="display:block;margin-bottom:10px">${question.question}</strong>
      ${question.options && question.options.length > 0 && html`
        <div class="task-toolbar" style="margin-bottom:10px">
          ${question.options.map((opt) => html`
            <button key=${opt} class="ghost" disabled=${busy} onClick=${() => submit(opt)}>${opt}</button>
          `)}
        </div>
      `}
      <form class="inline-form" onSubmit=${(e) => { e.preventDefault(); submit(answer); }}>
        <input type="text" placeholder="Ta réponse…" value=${answer}
          onInput=${(e) => setAnswer(e.target.value)} style="flex:1" disabled=${busy} />
        <button class="primary" type="submit" disabled=${busy || !answer.trim()}>Répondre</button>
      </form>
    </div>
  `;
}

function ToolRequestPanel({ projectId, request, onDone }) {
  const [busy, setBusy] = useState(false);

  const decide = async (decision) => {
    setBusy(true);
    try {
      await api.respondProjectToolRequest(projectId, {
        phase: request.phase, decision, tools: request.tools || [],
      });
      toast(decision === 'approve' ? 'Outils approuvés' : 'Outils refusés');
      await onDone();
    } catch (e) {
      toast(`Action impossible : ${e.message}`, 'err');
    } finally {
      setBusy(false);
    }
  };

  return html`
    <div class="ask-panel" style="margin-top:14px">
      <div class="item-row-meta" style="margin-bottom:6px">Demande d'autorisation d'outils (${PHASE_LABELS[request.phase] || request.phase})</div>
      <strong style="display:block;margin-bottom:6px">${(request.tools || []).join(', ')}</strong>
      ${request.reason && html`<p class="page-sub" style="margin:0 0 10px">${request.reason}</p>`}
      <div class="task-toolbar">
        <button class="primary" disabled=${busy} onClick=${() => decide('approve')}>Approuver</button>
        <button class="danger" disabled=${busy} onClick=${() => decide('deny')}>Refuser</button>
      </div>
    </div>
  `;
}
