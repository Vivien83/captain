import { h } from 'preact';
import { useState, useEffect, useRef, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api, openAgentWs } from '../api.js';
import { getState, setState, subscribe, toast } from '../store.js';
import { Markdown } from '../components/Markdown.js';
import { ToolCard } from '../components/ToolCard.js';
import { AskUserPrompt } from '../components/AskUserPrompt.js';
import {
  PROVIDER_QUOTA_REFRESH_MS,
  providerDurationLabel,
  providerQuotaGroups,
  providerQuotaMeta,
  providerQuotaTone,
  providerResetLabel,
  providerSubscriptionFromBudget,
} from '../provider_quota_model.mjs';

const html = htm.bind(h);

// Transcript items: {kind:'user'|'assistant'|'system', text, tools:[...], streaming}
let itemSeq = 0;
const newItem = (kind, text = '') => ({ id: ++itemSeq, kind, text, tools: [], streaming: false });

export function Chat() {
  const [items, setItems] = useState([]);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [canvas, setCanvas] = useState(null); // {title, html}
  const [agentId, setAgentId] = useState(getState().currentAgentId);
  const [activeModel, setActiveModel] = useState(activeModelIdentity(getState()));
  const [providerQuota, setProviderQuota] = useState(providerSubscriptionFromBudget(null));
  const wsRef = useRef(null);
  const scrollRef = useRef(null);
  const itemsRef = useRef(items);
  itemsRef.current = items;

  useEffect(() => subscribe((s) => {
    if (s.currentAgentId !== agentId) setAgentId(s.currentAgentId);
    setActiveModel(activeModelIdentity(s));
  }), [agentId]);

  // Captain's daemon owns provider calls and persistence. Web/desktop only
  // poll the local budget snapshot, exactly like the Ratatui status line.
  useEffect(() => {
    let dead = false;
    let timer = null;
    const refresh = async () => {
      try {
        const budget = await api.budget();
        if (!dead) setProviderQuota(providerSubscriptionFromBudget(budget));
      } catch {
        // Preserve the last provider-owned observation across a transient
        // daemon error; never turn missing data into an unlimited allowance.
      } finally {
        if (!dead) timer = setTimeout(refresh, PROVIDER_QUOTA_REFRESH_MS);
      }
    };
    refresh();
    return () => { dead = true; if (timer) clearTimeout(timer); };
  }, []);

  const mutate = useCallback((fn) => {
    setItems((prev) => {
      const next = prev.map((it) => ({ ...it, tools: it.tools.slice() }));
      fn(next);
      return next;
    });
  }, []);

  // Only continues an existing assistant bubble if it is the very last item —
  // a later user message (e.g. answering an onboarding question) means a
  // fresh turn, and must start a new bubble instead of appending after it.
  const lastAssistant = (list) => {
    const tail = list[list.length - 1];
    if (tail && tail.kind === 'assistant') return tail;
    const it = newItem('assistant');
    it.streaming = true;
    list.push(it);
    return it;
  };

  // Load past transcript for the active session, then connect the WS.
  useEffect(() => {
    if (!agentId) return;
    let dead = false;

    (async () => {
      try {
        const sessions = await api.agentSessions(agentId);
        const active = (sessions.sessions || sessions || []).find((s) => s.active) ||
          (sessions.sessions || sessions || [])[0];
        if (active && !dead) {
          setState({ currentSessionId: active.session_id });
          const ev = await api.sessionEvents(active.session_id);
          if (!dead) setItems(rebuildTranscript(ev.events || ev || []));
        }
      } catch { /* fresh session — empty transcript is fine */ }
    })();

    let ws = null;
    let closedByUs = false;
    let retry = 0;
    const connect = () => {
      ws = openAgentWs(agentId, {
        onopen: () => { retry = 0; setConnected(true); },
        onclose: () => {
          setConnected(false);
          if (!closedByUs) setTimeout(connect, Math.min(15000, 1000 * 2 ** retry++));
        },
        onmessage: (m) => handleWsMessage(m),
      });
      wsRef.current = ws;
    };
    connect();

    return () => { dead = true; closedByUs = true; if (ws) ws.close(); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId]);

  const handleWsMessage = (m) => {
    switch (m.type) {
      case 'typing':
        if (m.state === 'start') setBusy(true);
        if (m.state === 'stop') setBusy(false);
        break;
      case 'text_delta':
        mutate((list) => {
          const a = lastAssistant(list);
          a.streaming = true;
          a.text += m.content || '';
        });
        break;
      case 'tool_start':
        mutate((list) => {
          const a = lastAssistant(list);
          a.tools.push({ id: m.tool_use_id, name: m.tool, input: '', result: '', isError: false, done: false, startedAt: Date.now() });
        });
        break;
      case 'tool_end':
        mutate((list) => {
          const a = lastAssistant(list);
          const t = a.tools.find((t) => t.id === m.tool_use_id);
          if (t) t.input = m.input || t.input;
        });
        break;
      case 'tool_result':
        mutate((list) => {
          const a = lastAssistant(list);
          const t = a.tools.find((t) => t.id === m.tool_use_id);
          if (t) {
            t.result = m.result || '';
            t.isError = !!m.is_error;
            t.done = true;
            t.endedAt = Date.now();
          }
        });
        break;
      case 'response':
        setBusy(false);
        mutate((list) => {
          const a = lastAssistant(list);
          a.streaming = false;
          if (m.content && !a.text) a.text = m.content;
        });
        break;
      case 'error':
        setBusy(false);
        mutate((list) => { list.push({ ...newItem('system'), text: `Erreur : ${m.content}` }); });
        break;
      case 'ask_user':
        // Agent is blocked waiting on a human answer — stop the "thinking"
        // spinner (busy) and surface the question as its own item so it
        // doesn't get merged into an assistant bubble by lastAssistant().
        setBusy(false);
        mutate((list) => {
          // Defense in depth: the agent loop blocks on ask_user, so a second
          // question shouldn't arrive before the first is answered — but if
          // it did, two live button sets would both write to the same
          // backend channel. Close out any stale pending question first.
          list.forEach((it) => { if (it.kind === 'ask_user' && !it.answered) it.answered = true; });
          list.push({ ...newItem('ask_user'), text: m.question, options: m.options || null, answered: false });
        });
        break;
      case 'canvas':
        setCanvas({ title: m.title || 'Canvas', html: m.html || '' });
        break;
      case 'catch_up':
        if (m.is_streaming) {
          setBusy(true);
          mutate((list) => {
            if (m.user_message) list.push({ ...newItem('user'), text: m.user_message });
            const a = { ...newItem('assistant'), text: m.accumulated_text || '', streaming: true };
            list.push(a);
          });
        }
        break;
      case 'broadcast': {
        // Turn initiated from another surface (Telegram, TUI...) — mirror it.
        const ev = m.event || {};
        if (ev.UserMessage) mutate((l) => l.push({ ...newItem('user'), text: ev.UserMessage.content }));
        if (ev.TextDelta) mutate((l) => { const a = lastAssistant(l); a.streaming = true; a.text += ev.TextDelta.delta; });
        if (ev.ToolStart) mutate((l) => lastAssistant(l).tools.push({ id: ev.ToolStart.tool_use_id, name: ev.ToolStart.tool_name, input: '', result: '', isError: false, done: false, startedAt: Date.now() }));
        if (ev.ToolEnd) mutate((l) => {
          const t = lastAssistant(l).tools.find((t) => t.id === ev.ToolEnd.tool_use_id);
          if (t) { t.result = ev.ToolEnd.result_preview; t.isError = ev.ToolEnd.is_error; t.done = true; t.endedAt = Date.now(); }
        });
        if (ev.Response) mutate((l) => { const a = lastAssistant(l); a.streaming = false; if (!a.text && ev.Response.content) a.text = ev.Response.content; });
        break;
      }
      default: break;
    }
  };

  // Autoscroll pinned to bottom while streaming.
  useEffect(() => {
    const el = scrollRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 300) {
      el.scrollTop = el.scrollHeight;
    }
  }, [items]);

  // If the last item is an unanswered ask_user, free-text Composer input
  // must answer it too — same as clicking a button — instead of starting a
  // brand-new turn. ws.rs only routes `type:'user_response'` into the
  // waiting ask_user channel; `type:'message'` takes a different path that
  // the agent loop isn't listening on while blocked on ask_user.
  const pendingAskUser = () => {
    const list = itemsRef.current;
    const tail = list[list.length - 1];
    return (tail && tail.kind === 'ask_user' && !tail.answered) ? tail : null;
  };

  const answerAskUser = (item, answer) => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    mutate((list) => {
      const target = list.find((it) => it.id === item.id);
      if (target) { target.answered = true; target.answer = answer; }
    });
    ws.send(JSON.stringify({ type: 'user_response', content: answer }));
    setBusy(true);
  };

  const send = (text) => {
    const ws = wsRef.current;
    if (!text.trim() || !ws || ws.readyState !== WebSocket.OPEN) return false;
    const pending = pendingAskUser();
    if (pending) {
      answerAskUser(pending, text);
      return true;
    }
    setItems((prev) => [...prev, { ...newItem('user'), text }]);
    ws.send(JSON.stringify({ type: 'message', content: text }));
    setBusy(true);
    return true;
  };

  const onUpload = async (file) => {
    const fd = new FormData();
    fd.append('file', file);
    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(agentId)}/upload`, {
        method: 'POST', body: fd, credentials: 'same-origin',
      });
      if (!res.ok) throw new Error();
      const body = await res.json();
      toast(`Fichier envoyé : ${body.path || file.name}`);
      send(`J'ai uploadé un fichier : ${body.path || file.name}`);
    } catch {
      toast(`Échec de l'upload de ${file.name}`, 'err');
    }
  };

  return html`
    <div class="split">
      <div class="chat-col">
        <div class="chat-scroll" ref=${scrollRef}>
          <div class="chat-inner">
            ${items.length === 0 && html`
              <div class="empty-state">
                <div class="glyph">☰</div>
                <div>Parle à ton agent — il a 190+ outils à disposition.</div>
              </div>
            `}
            ${items.map((it) => html`<${Message} key=${it.id} item=${it} onAnswer=${answerAskUser} />`)}
          </div>
        </div>
        <${Composer} disabled=${!connected} busy=${busy} onSend=${send} onUpload=${onUpload} />
        <${ProviderQuotaBar} status=${providerQuota} activeModel=${activeModel} />
      </div>
      ${canvas && html`
        <div class="canvas-pane">
          <div class="head">
            <strong>${canvas.title}</strong>
            <span style="flex:1"></span>
            <button class="ghost" onClick=${() => setCanvas(null)}>✕</button>
          </div>
          <iframe sandbox="" srcdoc=${canvas.html}></iframe>
        </div>
      `}
    </div>
  `;
}

function ProviderQuotaBar({ status, activeModel }) {
  const groups = providerQuotaGroups(status, activeModel);
  const activeProvider = (activeModel || '').split('/')[0];
  const hasObservation = groups.hasProviderObservation;
  const codexActive = ['codex', 'openai-codex'].includes(activeProvider.toLowerCase());
  if (!hasObservation && !codexActive) return null;
  if (!hasObservation) {
    return html`
      <div class="provider-quota-bar unavailable" role="status">
        <strong>Codex</strong><span>quotas d'abonnement non observés</span>
      </div>
    `;
  }

  const meta = providerQuotaMeta(status, activeModel);
  const allWindows = groups.windows;
  const windows = allWindows.slice(0, 8);
  const alternativePressure = groups.alternativeTone === 'err'
    ? ' critique'
    : groups.alternativeTone === 'warn' ? ' sous tension' : '';
  return html`
    <div class="provider-quota-bar" role="status" aria-label="Quotas applicables au modèle actif ${meta.activeModel || meta.provider}">
      <div class="provider-quota-meta">
        <strong>${meta.activeModel ? `Actif : ${meta.activeModel}` : meta.provider}</strong>
        ${meta.activeModel && html`<span>${meta.provider}</span>`}
        ${meta.planType && html`<span class="provider-quota-plan">${meta.planType}</span>`}
        ${meta.creditsLabel && html`<span>${meta.creditsLabel}</span>`}
      </div>
      ${windows.map((window) => {
        const tone = providerQuotaTone(window);
        const duration = providerDurationLabel(
          window.windowSeconds,
          window.kind === 'primary' ? 'court' : 'long',
        );
        const percent = Number.isInteger(window.usedPercent)
          ? window.usedPercent.toFixed(0)
          : window.usedPercent.toFixed(1);
        return html`
          <div class="provider-quota-window ${tone}" key=${`${window.limitId}:${window.kind}`}>
            <span class="provider-quota-label">${window.limitName} · ${duration}</span>
            <span class="provider-quota-gauge" role="progressbar"
              aria-label="${window.limitName} ${duration}"
              aria-valuemin="0" aria-valuemax="100" aria-valuenow=${window.usedPercent}>
              <span style=${{ width: `${window.usedPercent}%` }}></span>
            </span>
            <strong>${percent}%</strong>
            <span class="provider-quota-reset">↻ ${providerResetLabel(window)}</span>
            ${window.stale && html`<span class="provider-quota-flag">stale</span>`}
            ${window.blocked && html`<span class="provider-quota-flag">bloqué</span>`}
          </div>
        `;
      })}
      ${allWindows.length > windows.length && html`
        <span class="provider-quota-more">+${allWindows.length - windows.length} fenêtre(s) applicable(s) dans Statut</span>
      `}
      ${groups.alternativeLimitCount > 0 && html`
        <span class="provider-quota-more ${groups.alternativeTone}">
          +${groups.alternativeLimitCount} quota${groups.alternativeLimitCount > 1 ? 's' : ''} annexe${groups.alternativeLimitCount > 1 ? 's' : ''}
          ${alternativePressure} · hors modèle actif · Statut
        </span>
      `}
    </div>
  `;
}

function activeModelIdentity(state) {
  const active = (state.agents || []).find((agent) => agent.id === state.currentAgentId);
  if (!active) return '';
  const provider = active.model_provider || '';
  const model = active.model_name || '';
  return provider && model ? `${provider}/${model}` : provider || model;
}

function Message({ item, onAnswer }) {
  const who = item.kind === 'user' ? 'Toi'
    : (item.kind === 'assistant' || item.kind === 'ask_user') ? 'Captain'
    : 'Système';
  return html`
    <div class="msg ${item.kind}">
      <div class="who">${who}</div>
      ${item.tools.map((t) => html`<${ToolCard} key=${t.id} tool=${t} />`)}
      ${(item.text || item.streaming) && html`
        <div class="bubble">
          <${Markdown} text=${item.text} />
          ${item.streaming && html`<span class="cursor-blink"></span>`}
        </div>
      `}
      ${item.kind === 'ask_user' && html`<${AskUserPrompt} item=${item} onAnswer=${onAnswer} />`}
    </div>
  `;
}

function Composer({ disabled, busy, onSend, onUpload }) {
  const [value, setValue] = useState('');
  const taRef = useRef(null);
  const [dragging, setDragging] = useState(false);

  const submit = () => {
    if (onSend(value)) {
      setValue('');
      if (taRef.current) taRef.current.style.height = 'auto';
    }
  };

  const onKey = (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  const autogrow = (e) => {
    setValue(e.target.value);
    e.target.style.height = 'auto';
    e.target.style.height = Math.min(190, e.target.scrollHeight) + 'px';
  };

  return html`
    <div class="composer-wrap ${dragging ? 'drop-active' : ''}"
      onDragOver=${(e) => { e.preventDefault(); setDragging(true); }}
      onDragLeave=${() => setDragging(false)}
      onDrop=${(e) => {
        e.preventDefault();
        setDragging(false);
        const f = e.dataTransfer.files && e.dataTransfer.files[0];
        if (f) onUpload(f);
      }}>
      <div class="composer">
        <textarea ref=${taRef} rows="1" value=${value}
          placeholder=${disabled ? 'Connexion au daemon…' : 'Écris à Captain…'}
          disabled=${disabled}
          onInput=${autogrow} onKeyDown=${onKey}></textarea>
        <button class="send primary" title="Envoyer" disabled=${disabled || busy || !value.trim()}
          onClick=${submit}>↑</button>
      </div>
      <div class="composer-hint">
        ${busy ? 'Captain travaille…' : 'Entrée pour envoyer · Maj+Entrée pour une nouvelle ligne · glisse un fichier ici'}
      </div>
    </div>
  `;
}

// Rebuild a transcript from persisted session events (same source as the TUI).
function rebuildTranscript(events) {
  const items = [];
  let current = null;
  let pendingAsk = null; // last ask_user item still waiting for its user_response in this replay
  for (const ev of events) {
    const type = ev.event_type || ev.type;
    const p = typeof ev.payload === 'string' ? safeParse(ev.payload) : (ev.payload || {});
    if (type === 'user_message') {
      items.push({ ...newItem('user'), text: p.content || p.text || '' });
      current = null;
    } else if (type === 'ask_user') {
      // timeline.rs persists this as event_type:"ask_user", payload:{question,options}
      // — mirror it into the same item shape handleWsMessage's live case builds,
      // so a reload shows the question (and its answer, once user_response
      // replays) instead of the pre-W4 gap where it silently vanished.
      const item = { ...newItem('ask_user'), text: p.question || '', options: p.options || null, answered: false };
      items.push(item);
      pendingAsk = item;
      current = null;
    } else if (type === 'user_response') {
      // timeline.rs persists this as event_type:"user_response", payload:{content}
      // — fold it into the ask_user item it answered rather than rendering
      // a second, separate item for the same exchange.
      if (pendingAsk) {
        pendingAsk.answered = true;
        pendingAsk.answer = p.content || '';
        pendingAsk = null;
      }
    } else if (type === 'assistant_message' || type === 'response') {
      current = { ...newItem('assistant'), text: p.content || p.text || '' };
      items.push(current);
    } else if (type === 'tool_use_start' || type === 'tool_use_end') {
      if (!current) { current = newItem('assistant'); items.push(current); }
      const id = p.tool_use_id || p.id || (p.input && p.input.tool_use_id) || `${type}-${items.length}-${current.tools.length}`;
      let t = current.tools.find((t) => t.id === id);
      if (!t) {
        t = { id, name: p.name || p.tool || 'tool', input: '', result: '', isError: false, done: false };
        current.tools.push(t);
      }
      if (type === 'tool_use_end' && p.input) t.input = JSON.stringify(p.input).slice(0, 500);
    } else if (type === 'tool_execution_result') {
      if (!current) { current = newItem('assistant'); items.push(current); }
      const t = current.tools.find((t) => !t.done);
      if (t) { t.result = p.result_preview || ''; t.isError = !!p.is_error; t.done = true; }
    }
  }
  return items;
}

function safeParse(s) {
  try { return JSON.parse(s); } catch { return {}; }
}
