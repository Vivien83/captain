import { h } from '/assets/app/vendor/preact.module.js';
import { useState, useEffect, useLayoutEffect, useRef, useCallback } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { api, openAgentWs } from '../api.js';
import { getState, setState, subscribe, toast } from '../store.js';
import { Markdown } from '../components/Markdown.js';
import { ToolCard } from '../components/ToolCard.js';
import { AskUserPrompt } from '../components/AskUserPrompt.js';
import { SuggestedReplies } from '../components/SuggestedReplies.js';
import {
  PROVIDER_QUOTA_REFRESH_MS,
  providerDurationLabel,
  providerQuotaGroups,
  providerQuotaMeta,
  providerQuotaTone,
  providerResetLabel,
  providerSubscriptionFromBudget,
} from '../provider_quota_model.mjs';
import {
  TextDeltaBatcher,
  isScrollNearBottom,
  textDeltaFromMessage,
} from '../chat_stream_batcher.mjs';
import { deliveryVerificationFromPhase } from '../verification_phase_model.mjs';

const html = htm.bind(h);

// Transcript items: {kind:'user'|'assistant'|'system', text, tools:[...], streaming}
let itemSeq = 0;
const newItem = (kind, text = '') => ({ id: ++itemSeq, kind, text, tools: [], streaming: false });

function lastAssistant(list) {
  const tail = list[list.length - 1];
  if (tail && tail.kind === 'assistant') return tail;
  const item = newItem('assistant');
  item.streaming = true;
  list.push(item);
  return item;
}

export function Chat() {
  const [items, setItems] = useState([]);
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [canvas, setCanvas] = useState(null); // {title, html}
  const [agentId, setAgentId] = useState(getState().currentAgentId);
  const [sessionId, setSessionId] = useState(getState().currentSessionId);
  const [activeModel, setActiveModel] = useState(activeModelIdentity(getState()));
  const [providerQuota, setProviderQuota] = useState(providerSubscriptionFromBudget(null));
  const [compaction, setCompaction] = useState(null);
  const [deliveryVerification, setDeliveryVerification] = useState(null);
  const [reasoning, setReasoning] = useState(null);
  const [reasoningBusy, setReasoningBusy] = useState(false);
  const wsRef = useRef(null);
  const scrollRef = useRef(null);
  const pinToBottomRef = useRef(true);
  const pinAfterRenderRef = useRef(false);
  const itemsRef = useRef(items);
  const compactionTimerRef = useRef(null);
  itemsRef.current = items;

  const preserveCompactionScrollPin = () => {
    const scroll = scrollRef.current;
    if (pinToBottomRef.current || (scroll && isScrollNearBottom(
      scroll.scrollHeight,
      scroll.scrollTop,
      scroll.clientHeight,
    ))) {
      pinAfterRenderRef.current = true;
    }
  };

  const applyCompactionProgress = (progress) => {
    if (!progress || !progress.operation_id) return;
    const activeSessionId = getState().currentSessionId;
    if (activeSessionId && progress.session_id && progress.session_id !== activeSessionId) return;
    if (compactionTimerRef.current) clearTimeout(compactionTimerRef.current);
    preserveCompactionScrollPin();
    setCompaction(progress);
    if (progress.state !== 'running') {
      compactionTimerRef.current = setTimeout(() => {
        preserveCompactionScrollPin();
        setCompaction(null);
      }, 6000);
    }
  };

  useEffect(() => subscribe((s) => {
    if (s.currentAgentId !== agentId) setAgentId(s.currentAgentId);
    if (s.currentSessionId !== sessionId) setSessionId(s.currentSessionId);
    setActiveModel(activeModelIdentity(s));
  }), [agentId, sessionId]);

  // Resolve an initial conversation once. Subsequent session selection stays
  // local to this Web client and never switches the agent-wide TUI/Telegram
  // registry entry.
  useEffect(() => {
    if (!agentId || sessionId) return undefined;
    let dead = false;
    (async () => {
      try {
        const response = await api.agentSessions(agentId);
        const sessions = response.sessions || response || [];
        let selected = sessions.find((session) => session.active) || sessions[0];
        if (!selected) selected = await api.createSession(agentId, { activate: false });
        if (!dead && selected?.session_id) {
          setState({ currentSessionId: selected.session_id });
        }
      } catch {
        // Shell refresh and reconnect will retry transient startup failures.
      }
    })();
    return () => { dead = true; };
  }, [agentId, sessionId]);

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

  useEffect(() => {
    let dead = false;
    if (!agentId || getState().clientMode === true) {
      setReasoning(null);
      return () => { dead = true; };
    }
    api.agentReasoning(agentId)
      .then((status) => { if (!dead) setReasoning(status); })
      .catch(() => { if (!dead) setReasoning(null); });
    return () => { dead = true; };
  }, [agentId, activeModel]);

  const setReasoningEffort = async (effort) => {
    if (!agentId || reasoningBusy) return;
    setReasoningBusy(true);
    try {
      const status = await api.setAgentReasoning(agentId, effort === 'auto' ? null : effort);
      setReasoning(status);
      const effective = status.effective_effort || 'provider';
      toast(`Raisonnement : ${effort} → ${effective}`);
    } catch (error) {
      toast(`Raisonnement refusé : ${error.message}`, 'err');
    } finally {
      setReasoningBusy(false);
    }
  };

  const mutate = useCallback((fn) => {
    const scroll = scrollRef.current;
    if (pinToBottomRef.current || (scroll && isScrollNearBottom(
      scroll.scrollHeight,
      scroll.scrollTop,
      scroll.clientHeight,
    ))) {
      // Keep the pre-mutation intent separate from onScroll. A delayed scroll
      // event can otherwise observe the larger DOM and mistake growth for an
      // operator scrolling away before Preact runs the layout effect.
      pinAfterRenderRef.current = true;
    }
    setItems((prev) => {
      const next = prev.map((it) => ({ ...it, tools: it.tools.slice() }));
      fn(next);
      return next;
    });
  }, []);

  const textBatcherRef = useRef(null);
  if (!textBatcherRef.current) {
    textBatcherRef.current = new TextDeltaBatcher((content) => {
      mutate((list) => {
        const assistant = lastAssistant(list);
        assistant.streaming = true;
        assistant.text += content;
      });
    });
  }
  const flushTextDeltas = () => textBatcherRef.current.flush();

  // Load and stream exactly the session selected by this Web client.
  useEffect(() => {
    if (!agentId || !sessionId) {
      setItems([]);
      setConnected(false);
      return undefined;
    }
    let dead = false;
    setCompaction(null);
    setDeliveryVerification(null);
    setItems([]);
    setConnected(false);
    pinToBottomRef.current = true;

    (async () => {
      try {
        const events = await api.sessionEvents(sessionId);
        if (!dead) setItems(rebuildTranscript(events.events || events || []));
      } catch { /* fresh session — empty transcript is fine */ }
    })();

    let ws = null;
    let closedByUs = false;
    let retry = 0;
    const connect = async () => {
      try {
        const opened = await openAgentWs(agentId, sessionId, {
          onopen: () => { retry = 0; setConnected(true); },
          onclose: () => {
            flushTextDeltas();
            setConnected(false);
            if (!closedByUs) setTimeout(connect, Math.min(15000, 1000 * 2 ** retry++));
          },
          onmessage: (m) => handleWsMessage(m),
        });
        if (closedByUs) {
          opened.close();
          return;
        }
        ws = opened;
        wsRef.current = ws;
      } catch {
        setConnected(false);
        if (!closedByUs) setTimeout(connect, Math.min(15000, 1000 * 2 ** retry++));
      }
    };
    connect();

    return () => {
      dead = true;
      closedByUs = true;
      if (compactionTimerRef.current) clearTimeout(compactionTimerRef.current);
      textBatcherRef.current.clear();
      if (ws) ws.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, sessionId]);

  const handleWsMessage = (m) => {
    const delta = textDeltaFromMessage(m);
    if (delta !== null) {
      textBatcherRef.current.push(delta);
      return;
    }
    // Preserve protocol ordering: every non-text boundary observes all text
    // received before it, even when the 34 ms visual frame has not elapsed.
    flushTextDeltas();
    switch (m.type) {
      case 'typing':
        if (m.state === 'start') setBusy(true);
        if (m.state === 'stop') setBusy(false);
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
        setDeliveryVerification(null);
        mutate((list) => {
          const a = lastAssistant(list);
          a.streaming = false;
          if (m.content && !a.text) a.text = m.content;
        });
        break;
      case 'error':
        setBusy(false);
        setDeliveryVerification(null);
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
      case 'suggested_replies':
        mutate((list) => {
          list.forEach((item) => { item.suggestionsActive = false; });
          const options = Array.isArray(m.options) ? m.options.filter((option) => typeof option === 'string' && option.trim()) : [];
          if (options.length > 0) {
            const assistant = lastAssistant(list);
            assistant.suggestedReplies = options;
            assistant.suggestionsActive = true;
          }
        });
        break;
      case 'compaction_progress':
        applyCompactionProgress(m.progress);
        break;
      case 'phase':
        setDeliveryVerification((current) =>
          deliveryVerificationFromPhase(m.phase, current));
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
        const payload = (name) => ev[name] || (ev.chat_event === name ? ev : null);
        const userMessage = payload('UserMessage');
        const textDelta = payload('TextDelta');
        const toolStart = payload('ToolStart');
        const toolEnd = payload('ToolEnd');
        const intermediate = payload('IntermediateMessage');
        const response = payload('Response');
        const askUser = payload('AskUser');
        const phase = payload('Phase');
        if (userMessage) {
          mutate((list) => {
            list.forEach((item) => { item.suggestionsActive = false; });
            list.push({ ...newItem('user'), text: userMessage.content });
          });
        }
        if (textDelta?.delta) mutate((list) => { lastAssistant(list).text += textDelta.delta; });
        if (toolStart) mutate((list) => lastAssistant(list).tools.push({ id: toolStart.tool_use_id, name: toolStart.tool_name, input: '', result: '', isError: false, done: false, startedAt: Date.now() }));
        if (toolEnd) mutate((list) => {
          const t = lastAssistant(list).tools.find((tool) => tool.id === toolEnd.tool_use_id);
          if (t) { t.result = toolEnd.result_preview; t.isError = toolEnd.is_error; t.done = true; t.endedAt = Date.now(); }
        });
        if (intermediate?.content) mutate((list) => { lastAssistant(list).text += intermediate.content; });
        if (response) mutate((list) => { const a = lastAssistant(list); a.streaming = false; if (!a.text && response.content) a.text = response.content; });
        if (phase) {
          setDeliveryVerification((current) =>
            deliveryVerificationFromPhase(phase.phase, current));
        }
        if (askUser) {
          setBusy(false);
          mutate((list) => {
            list.forEach((item) => { if (item.kind === 'ask_user' && !item.answered) item.answered = true; });
            list.push({ ...newItem('ask_user'), text: askUser.question, options: askUser.options || null, answered: false });
          });
        }
        const suggestedReplies = payload('SuggestedReplies')?.options;
        if (suggestedReplies) {
          mutate((list) => {
            list.forEach((item) => { item.suggestionsActive = false; });
            const options = suggestedReplies.filter((option) =>
              typeof option === 'string' && option.trim());
            if (options.length > 0) {
              const assistant = lastAssistant(list);
              assistant.suggestedReplies = options;
              assistant.suggestionsActive = true;
            }
          });
        }
        const compactionProgress = payload('CompactionProgress')?.progress;
        if (compactionProgress) applyCompactionProgress(compactionProgress);
        break;
      }
      default: break;
    }
  };

  // Preserve the operator's scrollback position, but keep a live session
  // pinned based on its position before a batched DOM growth.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && (pinToBottomRef.current || pinAfterRenderRef.current)) {
      el.scrollTop = el.scrollHeight;
      pinToBottomRef.current = true;
    }
    pinAfterRenderRef.current = false;
  }, [items, compaction]);

  const updateScrollPin = () => {
    const el = scrollRef.current;
    if (el) {
      pinToBottomRef.current = isScrollNearBottom(
        el.scrollHeight,
        el.scrollTop,
        el.clientHeight,
      );
    }
  };

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
    pinToBottomRef.current = true;
    ws.send(JSON.stringify({ type: 'user_response', content: answer }));
    setBusy(true);
  };

  const send = (text, attachments = []) => {
    const ws = wsRef.current;
    if (!text.trim() || !ws || ws.readyState !== WebSocket.OPEN) return false;
    const pending = pendingAskUser();
    if (pending) {
      answerAskUser(pending, text);
      return true;
    }
    pinToBottomRef.current = true;
    setItems((prev) => [
      ...prev.map((item) => item.suggestionsActive ? { ...item, suggestionsActive: false } : item),
      { ...newItem('user'), text },
    ]);
    ws.send(JSON.stringify({ type: 'message', content: text, attachments }));
    setBusy(true);
    return true;
  };

  const onUpload = async (file) => {
    const contentType = file.type || 'application/octet-stream';
    const filename = asciiFilename(file.name || 'attachment');
    try {
      const res = await fetch(`/api/agents/${encodeURIComponent(agentId)}/upload`, {
        method: 'POST',
        body: file,
        credentials: 'same-origin',
        headers: { 'content-type': contentType, 'x-filename': filename },
      });
      const body = await res.json();
      if (!res.ok) throw new Error(body.error || 'upload failed');
      toast(`Fichier envoyé : ${body.filename || file.name}`);
      const message = body.transcription
        ? `Message vocal transcrit depuis ${body.filename || file.name} :\n${body.transcription}`
        : `Analyse la pièce jointe ${body.filename || file.name}.`;
      send(message, [{
        file_id: body.file_id,
        filename: body.filename || file.name,
        content_type: body.content_type || contentType,
      }]);
    } catch (error) {
      toast(`Échec de l'upload de ${file.name} : ${error.message}`, 'err');
    }
  };

  return html`
    <div class="split">
      <div class="chat-col">
        <div class="chat-scroll" ref=${scrollRef} onScroll=${updateScrollPin}>
          <div class="chat-inner">
            ${items.length === 0 && html`
              <div class="empty-state">
                <div class="glyph">☰</div>
                <div>Parle à ton agent — il a 190+ outils à disposition.</div>
              </div>
            `}
            ${items.map((it) => html`<${Message} key=${it.id} item=${it}
              onAnswer=${answerAskUser} onSuggestedReply=${send} />`)}
          </div>
        </div>
        <${CompactionProgressBar} progress=${compaction} />
        <${DeliveryVerificationBar} status=${deliveryVerification} />
        <${Composer} disabled=${!connected} busy=${busy} onSend=${send} onUpload=${onUpload} />
        <${ProviderQuotaBar} status=${providerQuota} activeModel=${activeModel}
          reasoning=${getState().clientMode ? null : reasoning}
          reasoningBusy=${reasoningBusy}
          onReasoningChange=${getState().clientMode ? null : setReasoningEffort} />
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

function asciiFilename(filename) {
  return Array.from(filename || 'attachment')
    .map((character) => {
      const code = character.charCodeAt(0);
      return code >= 0x20 && code <= 0x7e ? character : '_';
    })
    .join('')
    .slice(0, 240) || 'attachment';
}

function DeliveryVerificationBar({ status }) {
  if (!status) return null;
  return html`
    <div class="delivery-verification ${status.phase}" role="status" aria-live="polite">
      <span class="delivery-verification-mark" aria-hidden="true"></span>
      <strong>${status.label}</strong>
      <span>${status.phase === 'correcting'
        ? 'Captain corrige uniquement les écarts observés avant de livrer.'
        : 'Captain contrôle les preuves utiles avant de confirmer le résultat.'}</span>
    </div>
  `;
}

function CompactionProgressBar({ progress }) {
  if (!progress) return null;
  const labels = {
    preparing: 'Préparation',
    pruning: 'Élagage',
    summarizing: 'Synthèse',
    chunking: 'Synthèse par lots',
    merging: 'Fusion',
    persisting: 'Enregistrement',
    completed: 'Terminé',
    failed: 'Échec',
    interrupted: 'Interrompu',
  };
  const completed = Number(progress.completed_units);
  const total = Number(progress.total_units);
  const determinate = Number.isFinite(completed) && Number.isFinite(total) && total > 0;
  const percent = determinate ? Math.max(0, Math.min(100, Math.floor((completed * 100) / total))) : null;
  const terminal = progress.state !== 'running';
  return html`
    <div class="compaction-progress ${terminal ? progress.state : 'running'}" role="status">
      <div class="compaction-progress-head">
        <strong>Compactage du contexte</strong>
        <span>${labels[progress.phase] || progress.phase}</span>
        ${determinate && html`<span>${completed}/${total} lots · ${percent}%</span>`}
        ${!determinate && !terminal && html`<span>progression indéterminée</span>`}
      </div>
      <div class="compaction-progress-gauge ${determinate ? '' : 'indeterminate'}"
        role="progressbar" aria-label="Progression du compactage"
        aria-valuemin=${determinate ? 0 : undefined}
        aria-valuemax=${determinate ? 100 : undefined}
        aria-valuenow=${determinate ? percent : undefined}>
        <span style=${determinate ? { width: `${percent}%` } : {}}></span>
      </div>
    </div>
  `;
}

function ProviderQuotaBar({ status, activeModel, reasoning, reasoningBusy, onReasoningChange }) {
  const groups = providerQuotaGroups(status, activeModel);
  const activeProvider = (activeModel || '').split('/')[0];
  const hasObservation = groups.hasProviderObservation;
  const codexActive = ['codex', 'openai-codex'].includes(activeProvider.toLowerCase());
  if (!hasObservation && !codexActive && !reasoning) return null;
  if (!hasObservation) {
    return html`
      <div class="provider-quota-bar unavailable" role="status">
        <${ReasoningControl} status=${reasoning} busy=${reasoningBusy}
          onChange=${onReasoningChange} />
        ${codexActive && html`<strong>Codex</strong><span>quotas d'abonnement non observés</span>`}
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
      <${ReasoningControl} status=${reasoning} busy=${reasoningBusy}
        onChange=${onReasoningChange} />
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
        const percent = Number.isInteger(window.remainingPercent)
          ? window.remainingPercent.toFixed(0)
          : window.remainingPercent.toFixed(1);
        return html`
          <div class="provider-quota-window ${tone}" key=${`${window.limitId}:${window.kind}`}>
            <span class="provider-quota-label">${window.limitName} · ${duration}</span>
            <span class="provider-quota-gauge" role="progressbar"
              aria-label="${window.limitName} ${duration} restant"
              aria-valuemin="0" aria-valuemax="100" aria-valuenow=${window.remainingPercent}>
              <span style=${{ width: `${window.remainingPercent}%` }}></span>
            </span>
            <strong>${percent}% reste</strong>
            <span class="provider-quota-reset">↻ ${providerResetLabel(window)}</span>
            ${window.stale && html`<span class="provider-quota-flag">stale</span>`}
            ${window.blocked && html`<span class="provider-quota-flag">bloqué</span>`}
          </div>
        `;
      })}
      ${groups.spendControls.map((control) => {
        const tone = providerQuotaTone(control);
        const percent = Number.isInteger(control.remainingPercent)
          ? control.remainingPercent.toFixed(0)
          : control.remainingPercent.toFixed(1);
        return html`
          <div class="provider-quota-window ${tone}" key=${`${control.limitId}:spend`}>
            <span class="provider-quota-label">Budget mensuel</span>
            <span class="provider-quota-gauge" role="progressbar"
              aria-label="Budget mensuel restant" aria-valuemin="0" aria-valuemax="100"
              aria-valuenow=${control.remainingPercent}>
              <span style=${{ width: `${control.remainingPercent}%` }}></span>
            </span>
            <strong>${percent}% reste</strong>
            <span>${control.remaining} / ${control.limit}</span>
            <span class="provider-quota-reset">↻ ${providerResetLabel(control)}</span>
            ${control.blocked && html`<span class="provider-quota-flag">bloqué</span>`}
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

function ReasoningControl({ status, busy, onChange }) {
  if (!status) return null;
  if (!status.supported) {
    return html`<div class="reasoning-control"><span>Raisonnement</span><strong>provider</strong></div>`;
  }
  const configured = status.configured_effort || 'auto';
  const effective = status.effective_effort || 'provider';
  const title = configured === 'ultra'
    ? 'Ultra utilise l’effort modèle max et active la délégation proactive sur l’agent racine'
    : configured === 'auto'
      ? `Auto omet l’override et laisse le modèle choisir (actuellement ${effective})`
      : configured === 'none'
        ? 'None est un niveau explicite, distinct de Auto'
        : 'Effort de raisonnement du modèle actif';
  return html`
    <label class="reasoning-control" title=${title}>
      <span>Raisonnement</span>
      <select value=${configured} disabled=${busy}
        onChange=${(event) => onChange(event.currentTarget.value)}>
        <option value="auto">${configured === 'auto' ? `Auto → ${effective}` : 'Auto'}</option>
        ${(status.options || []).map((option) => html`
          <option value=${option.effort} key=${option.effort}>${
            option.effort === 'ultra' ? 'Ultra · max + agents'
              : option.effort === 'none' ? 'None · explicite'
                : option.effort
          }</option>
        `)}
      </select>
    </label>
  `;
}

function activeModelIdentity(state) {
  const active = (state.agents || []).find((agent) => agent.id === state.currentAgentId);
  if (!active) return '';
  const provider = active.model_provider || '';
  const model = active.model_name || '';
  return provider && model ? `${provider}/${model}` : provider || model;
}

function Message({ item, onAnswer, onSuggestedReply }) {
  const who = item.kind === 'user' ? 'Toi'
    : (item.kind === 'assistant' || item.kind === 'ask_user') ? 'Captain'
    : 'Système';
  return html`
    <div class="msg ${item.kind} ${item.animate === false ? 'settled' : ''} ${item.streaming ? 'streaming' : ''}">
      <div class="who">${who}</div>
      ${item.tools.map((t) => html`<${ToolCard} key=${t.id} tool=${t} />`)}
      ${(item.text || item.streaming) && html`
        <div class="bubble">
          <${Markdown} text=${item.text} />
          ${item.streaming && html`<span class="cursor-blink"></span>`}
        </div>
      `}
      ${item.kind === 'ask_user' && html`<${AskUserPrompt} item=${item} onAnswer=${onAnswer} />`}
      ${item.kind === 'assistant' && html`
        <${SuggestedReplies} item=${item}
          onChoose=${(_item, option) => onSuggestedReply(option)} />
      `}
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
  let pendingSuggestions = null;
  for (const ev of events) {
    const type = ev.event_type || ev.type;
    const p = typeof ev.payload === 'string' ? safeParse(ev.payload) : (ev.payload || {});
    if (type === 'user_message') {
      items.forEach((item) => { item.suggestionsActive = false; });
      pendingSuggestions = null;
      items.push({ ...newItem('user'), text: p.content || p.text || '' });
      current = null;
    } else if (type === 'suggested_replies') {
      items.forEach((item) => { item.suggestionsActive = false; });
      const options = Array.isArray(p.options)
        ? p.options.filter((option) => typeof option === 'string' && option.trim())
        : [];
      pendingSuggestions = options.length > 0 ? options : null;
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
      if (pendingSuggestions) {
        current.suggestedReplies = pendingSuggestions;
        current.suggestionsActive = true;
        pendingSuggestions = null;
      }
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
  return items.map((item) => ({ ...item, animate: false }));
}

function safeParse(s) {
  try { return JSON.parse(s); } catch { return {}; }
}
