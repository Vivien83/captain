// Captain Web Terminal — xterm.js + native PTY WebSocket bridge
'use strict';

(function() {
  var terminalEl = document.getElementById('terminal');
  if (!terminalEl || typeof Terminal === 'undefined') return;

  var fitAddon = typeof FitAddon !== 'undefined' && FitAddon.FitAddon
    ? new FitAddon.FitAddon()
    : null;
  var unicode11Addon = typeof Unicode11Addon !== 'undefined' && Unicode11Addon.Unicode11Addon
    ? new Unicode11Addon.Unicode11Addon()
    : null;
  if (!unicode11Addon) {
    terminalEl.textContent = 'Captain terminal Unicode support failed to load.';
    return;
  }
  var sessionInput = document.getElementById('session-id');
  var connectBtn = document.getElementById('connect');
  var terminateBtn = document.getElementById('terminate');
  var statusDot = document.getElementById('status-dot');
  var statusText = document.getElementById('status-text');
  var sessionOptions = document.getElementById('session-options');
  var workbench = document.querySelector('.terminal-workbench');
  var newSessionBtn = document.getElementById('new-session');
  var sessionsToggle = document.getElementById('sessions-toggle');
  var activityToggle = document.getElementById('activity-toggle');
  var commandToggle = document.getElementById('command-toggle');
  var commandBar = document.getElementById('command-bar');
  var commandInput = document.getElementById('command-input');
  var commandButtons = document.querySelectorAll('[data-command]');
  var attachmentButton = document.getElementById('attachment-button');
  var attachmentInput = document.getElementById('attachment-input');
  var callToggleBtn = document.getElementById('call-toggle');
  var callEndBtn = document.getElementById('call-end');
  var refreshSessionsBtn = document.getElementById('refresh-sessions');
  var sessionList = document.getElementById('session-list');
  var activityList = document.getElementById('activity-list');
  var activityCount = document.getElementById('activity-count');
  var metricTokens = document.getElementById('metric-tokens');
  var metricCost = document.getElementById('metric-cost');
  var metricCalls = document.getElementById('metric-calls');
  var browserPreview = document.getElementById('browser-preview');
  var browserPreviewText = document.getElementById('browser-preview-text');
  var replayChip = document.getElementById('replay-chip');
  var callChip = document.getElementById('call-chip');
  var callMeter = document.getElementById('call-meter');
  var callStatus = document.getElementById('call-status');
  var callSpectrumCanvas = document.getElementById('call-spectrum');
  var voiceTranscript = document.getElementById('voice-transcript');
  var voiceTranscriptList = document.getElementById('voice-transcript-list');
  var voiceTranscriptClear = document.getElementById('voice-transcript-clear');
  var authPanel = document.getElementById('auth-panel');
  var authForm = document.getElementById('auth-form');
  var authHelp = document.getElementById('auth-help');
  var sessionFields = document.getElementById('session-fields');
  var usernameInput = document.getElementById('auth-username');
  var passwordInput = document.getElementById('auth-password');
  var placeholderEl = document.getElementById('terminal-placeholder');
  var ws = null;
  var connected = false;
  var connecting = false;
  var mode = 'captain';
  var authMode = 'unknown';
  var autoSession = false;
  var attachRetryCount = 0;
  var autoSessionKey = 'captain.web_chat.session_id';
  var recentSessionsKey = 'captain.web_chat.recent_sessions';
  var recentSessionsLimit = 18;
  var knownSessionItems = [];
  var mainSessionId = 'main';
  var activeResumeSessionId = null;
  var newSessionPending = false;
  var activity = [];
  var maxActivity = 42;
  var lastActivityKey = '';
  var lastActivityAt = 0;
  var sessionEventCursor = Date.now() - 60000;
  var sessionEventSeen = {};
  var sessionEventPollTimer = 0;
  var responsiveMode = null;
  var cachedAgentId = null;
  var terminalFontSize = null;
  var terminalTouchY = null;
  var terminalTouchAccum = 0;
  var terminalPointerId = null;
  var terminalInputLine = '';
  var terminalInputLineAt = 0;
  var terminalCompositionActive = false;
  var terminalHelperTextarea = null;
  var callPc = null;
  var callDataChannel = null;
  var callMediaStream = null;
  var callAudioEl = null;
  var callAudioContext = null;
  var callAnalyser = null;
  var callMicSource = null;
  var callSpectrumFrame = 0;
  var callSpectrumData = null;
  var callTimeData = null;
  var callWatchdogTimer = 0;
  var callLastSoundAt = 0;
  var callLastRealtimeAt = 0;
  var callAutoEndSilenceMs = 90000;
  var callAutoEndInactiveMs = 180000;
  var callSoundThreshold = 0.035;
  var callConnecting = false;
  var callActive = false;
  var callMicEnabled = false;
  var callPushToTalkDown = false;
  var callPointerId = null;
  var handledCallToolIds = {};
  var voiceMirrorSeq = 0;
  var voiceEvents = [];
  var maxVoiceEvents = 18;

  var term = new Terminal({
    allowProposedApi: true,
    cursorBlink: true,
    convertEol: false,
    fontFamily: "'Geist Mono', 'SF Mono', 'Fira Code', 'Cascadia Code', monospace",
    fontSize: 13,
    lineHeight: 1.15,
    scrollback: 5000,
    theme: {
      background: '#030706',
      foreground: '#d8ffe8',
      cursor: '#BFFD00',
      cursorAccent: '#0a0a0a',
      selectionBackground: '#263300',
      black: '#0a0a0a',
      red: '#ff6b6b',
      green: '#84e8b0',
      yellow: '#f5c842',
      blue: '#5ab4d6',
      magenta: '#c084fc',
      cyan: '#67e8f9',
      white: '#f2f0eb',
      brightBlack: '#555555',
      brightRed: '#ff8c8c',
      brightGreen: '#a8f2c9',
      brightYellow: '#ffe17a',
      brightBlue: '#91d6ef',
      brightMagenta: '#d8b4fe',
      brightCyan: '#9cf6ff',
      brightWhite: '#fff6d6'
    }
  });

  term.loadAddon(unicode11Addon);
  term.unicode.activeVersion = '11';
  if (fitAddon) term.loadAddon(fitAddon);
  term.open(terminalEl);
  configureTerminalKeyboardInput();

  function setStatus(text, state) {
    statusText.textContent = text;
    statusDot.classList.toggle('connected', state === 'connected');
    statusDot.classList.toggle('error', state === 'error');
    document.body.dataset.terminalState = state || 'idle';
  }

  function setPlaceholder(text, visible) {
    if (!placeholderEl) return;
    if (text) placeholderEl.textContent = text;
    placeholderEl.hidden = visible === false;
  }

  function terminalHelperInput() {
    if (terminalHelperTextarea && document.body.contains(terminalHelperTextarea)) {
      return terminalHelperTextarea;
    }
    terminalHelperTextarea = terminalEl.querySelector('textarea.xterm-helper-textarea')
      || terminalEl.querySelector('.xterm-helper-textarea');
    return terminalHelperTextarea;
  }

  function clearTerminalHelperInputSoon() {
    window.setTimeout(function() {
      if (terminalCompositionActive) return;
      var helper = terminalHelperInput();
      if (helper && helper.value) helper.value = '';
    }, 0);
  }

  function configureTerminalKeyboardInput() {
    var helper = terminalHelperInput();
    if (!helper || helper.dataset.captainKeyboardConfigured === 'true') return;
    helper.dataset.captainKeyboardConfigured = 'true';
    helper.setAttribute('autocomplete', 'off');
    helper.setAttribute('autocorrect', 'off');
    helper.setAttribute('autocapitalize', 'none');
    helper.setAttribute('spellcheck', 'false');
    helper.setAttribute('data-gramm', 'false');
    helper.setAttribute('data-lpignore', 'true');
    helper.addEventListener('compositionstart', function() {
      terminalCompositionActive = true;
    });
    helper.addEventListener('compositionend', function() {
      terminalCompositionActive = false;
      clearTerminalHelperInputSoon();
    });
    helper.addEventListener('blur', function() {
      terminalCompositionActive = false;
      if (helper.value) helper.value = '';
    });
  }

  function validSessionId(value) {
    return /^[A-Za-z0-9._-]{1,80}$/.test(value || '');
  }

  function validUuid(value) {
    return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value || '');
  }

  function projectSessionFromSlug(value) {
    var slug = String(value || '')
      .trim()
      .replace(/[^A-Za-z0-9._-]/g, '-')
      .replace(/-+/g, '-')
      .slice(0, 64);
    return slug ? 'project-' + slug : '';
  }

  function escapeHtml(value) {
    return String(value || '').replace(/[&<>"']/g, function(ch) {
      return ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch];
    });
  }

  function stripAnsi(value) {
    return String(value || '')
      .replace(/\x1B\[[0-?]*[ -/]*[@-~]/g, '')
      .replace(/\x1B\][^\x07]*(\x07|\x1B\\)/g, '');
  }

  function shortText(value, max) {
    var text = stripAnsi(value).replace(/\s+/g, ' ').trim();
    if (text.length <= max) return text;
    return text.slice(0, max - 1) + '…';
  }

  function terminalSafeText(value, max) {
    var text = stripAnsi(value)
      .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')
      .replace(/\r\n/g, '\n')
      .replace(/\r/g, '\n')
      .trim();
    if (max && text.length > max) text = text.slice(0, max - 1) + '…';
    return text;
  }

  function mirrorVoiceToTerminal(role, text, options) {
    var body = terminalSafeText(text, options && options.max || 6000);
    if (!body) return;
    var label = role === 'voice' ? 'voice'
      : role === 'captain' ? 'captain'
      : role === 'error' ? 'error'
      : 'call';
    voiceEvents.push({
      role: role || 'status',
      label: label,
      sequence: options && options.sequence ? options.sequence : null,
      text: body,
      pending: !!(options && options.pending),
      at: new Date()
    });
    if (voiceEvents.length > maxVoiceEvents) {
      voiceEvents.splice(0, voiceEvents.length - maxVoiceEvents);
    }
    setPlaceholder('', false);
    renderVoiceTranscript();
  }

  function renderVoiceTranscript() {
    if (!voiceTranscript || !voiceTranscriptList) return;
    voiceTranscript.hidden = voiceEvents.length === 0;
    if (!voiceEvents.length) {
      voiceTranscriptList.innerHTML = '';
      window.requestAnimationFrame(fitAndResize);
      return;
    }
    voiceTranscriptList.innerHTML = voiceEvents.map(function(item) {
      var seq = item.sequence ? ' #' + item.sequence : '';
      var pending = item.pending ? ' ...' : '';
      return [
        '<article class="terminal-voice-line kind-' + escapeHtml(item.role) + '">',
        '<div class="terminal-voice-meta"><span>' + escapeHtml(item.label + seq) + '</span><span>' + item.at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) + '</span></div>',
        '<div class="terminal-voice-text">' + escapeHtml(item.text + pending) + '</div>',
        '</article>'
      ].join('');
    }).join('');
    voiceTranscriptList.scrollTop = voiceTranscriptList.scrollHeight;
    window.requestAnimationFrame(fitAndResize);
  }

  function summarizeVoiceEvents(limit) {
    var n = Number(limit || 8);
    if (!Number.isFinite(n) || n <= 0) n = 8;
    var items = voiceEvents.slice(-Math.min(20, n));
    if (!items.length) return 'No recent Captain voice activity is available yet.';
    return items.map(function(item) {
      var seq = item.sequence ? ' #' + item.sequence : '';
      return '[' + item.label + seq + '] ' + item.text;
    }).join('\n');
  }

  function formatCompactNumber(value) {
    var n = Number(value || 0);
    if (n >= 1000000000) return (n / 1000000000).toFixed(1).replace(/\.0$/, '') + 'B';
    if (n >= 1000000) return (n / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
    return String(Math.max(0, Math.round(n)));
  }

  function formatCost(value) {
    var n = Number(value || 0);
    if (n <= 0) return '$0';
    if (n < 0.01) return '$' + n.toFixed(4);
    if (n < 10) return '$' + n.toFixed(2);
    return '$' + Math.round(n);
  }

  function formatSessionDate(value) {
    if (!value) return '';
    var date = new Date(value);
    if (Number.isNaN(date.getTime())) return '';
    return date.toLocaleDateString([], { month: 'short', day: '2-digit' }) + ' ' +
      date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  function resetActivity() {
    activity = [];
    lastActivityKey = '';
    lastActivityAt = 0;
    if (browserPreview) browserPreview.hidden = true;
    if (browserPreviewText) browserPreviewText.textContent = 'Waiting for browser activity';
    renderActivity();
  }

  function makeAutoSessionId() {
    var suffix = '';
    if (window.crypto && window.crypto.getRandomValues) {
      var bytes = new Uint8Array(4);
      window.crypto.getRandomValues(bytes);
      for (var i = 0; i < bytes.length; i++) {
        suffix += bytes[i].toString(16).padStart(2, '0');
      }
    } else {
      suffix = Math.random().toString(16).slice(2, 10);
    }
    return 'web-' + Date.now().toString(36) + '-' + suffix;
  }

  function syncSessionUrl(value) {
    if (!validSessionId(value) || !window.history || !window.history.replaceState) return;
    var url = new URL(window.location.href);
    var resume = activeResumeSessionId && validUuid(activeResumeSessionId) ? activeResumeSessionId : '';
    if (url.searchParams.get('session') === value && (url.searchParams.get('resume_session') || '') === resume) return;
    url.searchParams.set('session', value);
    if (resume) {
      url.searchParams.set('resume_session', resume);
    } else {
      url.searchParams.delete('resume_session');
    }
    window.history.replaceState(null, '', url.pathname + url.search + url.hash);
  }

  function rotateAutoSessionId() {
    activeResumeSessionId = null;
    var value = makeAutoSessionId();
    window.sessionStorage.setItem(autoSessionKey, value);
    sessionInput.value = value;
    autoSession = true;
    syncSessionUrl(value);
    return value;
  }

  function readRecentSessions() {
    try {
      var raw = JSON.parse(window.localStorage.getItem(recentSessionsKey) || '[]');
      if (!Array.isArray(raw)) return [];
      return raw.filter(function(value) { return validSessionId(value); });
    } catch(e) {
      return [];
    }
  }

  function writeRecentSessions(values) {
    window.localStorage.setItem(
      recentSessionsKey,
      JSON.stringify(values.slice(0, recentSessionsLimit))
    );
  }

  function normalizeSessionItems(values) {
    var seen = {};
    var items = [];
    values.forEach(function(item) {
      var id = typeof item === 'string' ? item : item && (item.id || item.session_id);
      if (!validSessionId(id) || seen[id]) return;
      seen[id] = true;
      var source = item && item.source || (item && item.session_id ? 'history' : (item && item.local ? 'local' : 'terminal'));
      var restorable = item && item.restorable;
      if (restorable === undefined) {
        restorable = !(source === 'local' && id !== mainSessionId && Number(item && item.replay_bytes || 0) <= 0);
      }
      items.push({
        id: id,
        mode: (item && item.mode) || 'captain',
        active_clients: Number(item && item.active_clients || 0),
        replay_bytes: Number(item && item.replay_bytes || 0),
        local: !!(item && item.local),
        source: source,
        restorable: !!restorable,
        resume_session: item && (item.resume_session || item.session_id) || '',
        agent_id: item && item.agent_id || '',
        agent_name: item && item.agent_name || '',
        label: item && item.label || '',
        message_count: Number(item && item.message_count || 0),
        context_window_tokens: Number(item && item.context_window_tokens || 0),
        active: !!(item && item.active),
        updated_at: item && (item.updated_at || item.last_active || item.created_at) || ''
      });
    });
    return items;
  }

  function normalizeAgentSessionItems(values) {
    return normalizeSessionItems((values || []).map(function(item) {
      var sessionId = item && (item.session_id || item.id);
      return {
        id: sessionId,
        session_id: sessionId,
        source: 'history',
        mode: 'captain',
        active_clients: 0,
        replay_bytes: 0,
        local: false,
        restorable: validUuid(sessionId),
        resume_session: sessionId,
        agent_id: item && item.agent_id || '',
        agent_name: item && item.agent_name || '',
        label: item && item.label || '',
        message_count: Number(item && item.message_count || 0),
        context_window_tokens: Number(item && item.context_window_tokens || 0),
        active: !!(item && item.active),
        updated_at: item && (item.updated_at || item.last_active || item.created_at) || ''
      };
    }));
  }

  function localSessionItems(remoteValues, historyValues) {
    var remote = normalizeSessionItems(remoteValues || []);
    var history = normalizeAgentSessionItems(historyValues || []);
    var remoteById = {};
    remote.forEach(function(item) { remoteById[item.id] = item; });
    var historyById = {};
    history.forEach(function(item) { historyById[item.id] = item; });
    var seen = {};
    var items = [];

    function push(id) {
      if (!validSessionId(id) || seen[id]) return;
      seen[id] = true;
      // A legacy PTY may share the canonical UUID. Prefer the persisted row so
      // reopening always starts a fresh wrapper against the latest transcript.
      items.push(historyById[id] || remoteById[id] || {
        id: id,
        mode: 'captain',
        active_clients: 0,
        replay_bytes: 0,
        local: true,
        source: 'local',
        restorable: id === mainSessionId,
        resume_session: '',
        agent_id: '',
        agent_name: '',
        label: '',
        message_count: 0,
        context_window_tokens: 0,
        active: false,
        updated_at: ''
      });
    }

    push(mainSessionId);
    readRecentSessions().forEach(push);
    history.forEach(function(item) {
      if (seen[item.id]) return;
      seen[item.id] = true;
      items.push(item);
    });
    remote.forEach(function(item) {
      if (seen[item.id]) return;
      seen[item.id] = true;
      items.push(item);
    });
    return items;
  }

  function renderSessionOptions(values) {
    if (!sessionOptions) return;
    var items = normalizeSessionItems(values);
    sessionOptions.innerHTML = '';
    items.forEach(function(item) {
      var option = document.createElement('option');
      option.value = item.id;
      sessionOptions.appendChild(option);
    });
  }

  function renderSessionList(values) {
    if (!sessionList) return;
    var current = sessionInput.value;
    var items = normalizeSessionItems(values);
    if (!items.length) {
      sessionList.innerHTML = '<div class="terminal-empty">No sessions yet</div>';
      return;
    }
    sessionList.innerHTML = items.map(function(item) {
      var live = item.active_clients > 0;
      var isHistory = item.source === 'history';
      var isStaleLocal = item.source === 'local' && !item.restorable;
      var replay = item.replay_bytes ? Math.max(1, Math.round(item.replay_bytes / 1024)) + 'k replay'
        : isHistory ? item.message_count + ' msg'
        : isStaleLocal ? 'no replay'
        : 'fresh';
      var status = live ? 'attached'
        : isHistory ? 'stored'
        : isStaleLocal ? 'not restorable'
        : 'available';
      var source = isHistory ? 'history'
        : item.source === 'local' ? 'browser'
        : 'terminal';
      var owner = item.agent_name || (item.agent_id ? 'agent ' + item.agent_id.slice(0, 8) : '');
      var label = item.label || item.id;
      var updated = formatSessionDate(item.updated_at);
      var canTerminate = item.source === 'terminal';
      return [
        '<div class="terminal-session-row">',
        '<button type="button" class="terminal-session-card' + (item.id === current ? ' active' : '') + (isHistory ? ' history' : '') + (isStaleLocal ? ' stale' : '') + '" data-session="' + escapeHtml(item.id) + '" data-active-clients="' + item.active_clients + '" data-replay-bytes="' + item.replay_bytes + '" data-local="' + (item.local ? 'true' : 'false') + '" data-source="' + escapeHtml(item.source) + '" data-mode="' + escapeHtml(item.mode) + '" data-restorable="' + (item.restorable ? 'true' : 'false') + '" data-resume-session="' + escapeHtml(item.resume_session || '') + '">',
        '<span class="terminal-session-dot ' + (live ? 'live' : '') + (isHistory ? ' history' : '') + '"></span>',
        '<span class="terminal-session-name">' + escapeHtml(label) + '</span>',
        '<span class="terminal-session-meta"><span>' + escapeHtml(source) + '</span>' + (owner ? '<span>' + escapeHtml(owner) + '</span>' : '') + '<span>' + escapeHtml(status) + '</span><span>' + escapeHtml(replay) + '</span>' + (updated ? '<span>' + escapeHtml(updated) + '</span>' : '') + '</span>',
        '</button>',
        canTerminate ? '<button type="button" class="terminal-session-kill" title="Terminate session" aria-label="Terminate session ' + escapeHtml(item.id) + '" data-terminate-session="' + escapeHtml(item.id) + '" data-mode="' + escapeHtml(item.mode) + '">×</button>' : '',
        '</div>'
      ].join('');
    }).join('');
  }

  function rememberSessionId(value) {
    if (!validSessionId(value)) return;
    var sessions = readRecentSessions().filter(function(item) { return item !== value; });
    sessions.unshift(value);
    writeRecentSessions(sessions);
  }

  function forgetSessionId(value) {
    if (!validSessionId(value)) return;
    writeRecentSessions(readRecentSessions().filter(function(item) { return item !== value; }));
  }

  function loadTerminalSessions() {
    var local = localSessionItems([], []);
    renderSessionOptions(local);
    renderSessionList(local);
    var livePromise = fetch('/api/terminal/sessions', { credentials: 'same-origin' })
      .then(function(r) { return r.ok ? r.json() : { sessions: [] }; })
      .then(function(payload) { return Array.isArray(payload.sessions) ? payload.sessions : []; })
      .catch(function() { return []; });
    var historyPromise = fetch('/api/sessions', { credentials: 'same-origin' })
      .then(function(r) { return r.ok ? r.json() : { sessions: [] }; })
      .then(function(payload) { return Array.isArray(payload.sessions) ? payload.sessions : []; })
      .catch(function() { return []; });

    return Promise.all([livePromise, historyPromise]).then(function(results) {
      var combined = localSessionItems(results[0], results[1]);
      knownSessionItems = combined;
      renderSessionOptions(combined);
      renderSessionList(combined);
      return combined;
    });
  }

  function remoteHasSession(items, id) {
    return normalizeSessionItems(items).some(function(item) {
      return item.id === id && item.source !== 'local';
    });
  }

  function hasStaleAutoSession(items) {
    var current = (sessionInput.value || '').trim();
    return autoSession
      && !(activeResumeSessionId && validUuid(activeResumeSessionId))
      && validSessionId(current)
      && !remoteHasSession(items, current);
  }

  function bindKnownPersistedSession(items) {
    var current = (sessionInput.value || '').trim();
    if (!validSessionId(current)) return;
    if (activeResumeSessionId && validUuid(activeResumeSessionId)) {
      if (current === activeResumeSessionId) {
        current = makeAutoSessionId();
        sessionInput.value = current;
        autoSession = true;
        window.sessionStorage.setItem(autoSessionKey, current);
        rememberSessionId(current);
        syncSessionUrl(current);
      }
      return;
    }
    var normalized = normalizeSessionItems(items);
    var history = normalized.filter(function(item) {
      return item.source === 'history' && validUuid(item.resume_session || item.id);
    });
    var matched = history.find(function(item) { return item.id === current; });
    if (matched) {
      activeResumeSessionId = matched.resume_session || matched.id;
      current = makeAutoSessionId();
      sessionInput.value = current;
      autoSession = true;
      window.sessionStorage.setItem(autoSessionKey, current);
      rememberSessionId(current);
      syncSessionUrl(current);
      return;
    }
    var liveTerminal = normalized.some(function(item) {
      return item.id === current && item.source === 'terminal';
    });
    if (liveTerminal) return;
    syncSessionUrl(current);
  }

  function ensureInitialSessionId() {
    var params = new URLSearchParams(window.location.search);
    var resumeSession = (params.get('resume_session') || '').trim();
    activeResumeSessionId = validUuid(resumeSession) ? resumeSession : activeResumeSessionId;
    var querySession = (params.get('session') || '').trim();
    if (validSessionId(querySession)) {
      sessionInput.value = querySession;
      autoSession = querySession.indexOf('web-') === 0;
      if (autoSession) window.sessionStorage.setItem(autoSessionKey, querySession);
      rememberSessionId(querySession);
      return querySession;
    }

    var queryProject = (params.get('project') || '').trim();
    var projectSession = projectSessionFromSlug(queryProject);
    if (validSessionId(projectSession)) {
      sessionInput.value = projectSession;
      autoSession = false;
      syncSessionUrl(projectSession);
      rememberSessionId(projectSession);
      return projectSession;
    }

    var current = (sessionInput.value || '').trim();
    if (current && validSessionId(current)) {
      autoSession = current.indexOf('web-') === 0;
      syncSessionUrl(current);
      rememberSessionId(current);
      return current;
    }

    sessionInput.value = mainSessionId;
    autoSession = false;
    activeResumeSessionId = null;
    syncSessionUrl(mainSessionId);
    rememberSessionId(mainSessionId);
    return mainSessionId;
  }

  function showAuthPanel(kind, message) {
    if (!authPanel) return;
    authMode = kind || 'session';
    authHelp.textContent = message || 'Sign in to open the terminal.';
    sessionFields.hidden = authMode !== 'session';
    authPanel.hidden = false;
    setPlaceholder('Sign in to open Captain chat.', true);
    window.requestAnimationFrame(function() {
      if (authMode === 'session' && usernameInput) usernameInput.focus();
    });
  }

  function hideAuthPanel() {
    if (authPanel) authPanel.hidden = true;
  }

  function sessionId() {
    var value = (sessionInput.value || mainSessionId).trim();
    if (!value) {
      return ensureInitialSessionId();
    }
    if (value === mainSessionId) autoSession = false;
    if (!validSessionId(value)) {
      value = rotateAutoSessionId();
    }
    rememberSessionId(value);
    return value;
  }

  function terminalUrl() {
    var proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    var params = new URLSearchParams();
    params.set('mode', mode);
    params.set('rows', String(term.rows || 30));
    params.set('cols', String(term.cols || 100));
    if (activeResumeSessionId && validUuid(activeResumeSessionId)) {
      params.set('resume_session', activeResumeSessionId);
    }
    return proto + '//' + window.location.host + '/api/sessions/' +
      encodeURIComponent(sessionId()) + '/terminal?' + params.toString();
  }

  function switchSession(id, meta) {
    if (!validSessionId(id)) return;
    meta = meta || {};
    if (meta.restorable === false) {
      setStatus('session history unavailable:' + id, 'error');
      setPlaceholder('This browser-only terminal id has no server replay. Open a persisted history session or create a new chat.', true);
      addActivity('warning', 'Session history unavailable', id);
      term.focus();
      return;
    }
    if (id === sessionInput.value && connected) {
      addActivity('success', 'Session already open', id);
      term.focus();
      return;
    }
    activeResumeSessionId = validUuid(meta.resume_session || '') ? meta.resume_session : null;
    if (!activeResumeSessionId && validUuid(id) && meta.source === 'history') {
      activeResumeSessionId = id;
    }
    if (Number(meta.active_clients || 0) > 0 && id !== sessionInput.value && meta.source !== 'history') {
      setStatus('session already attached:' + id, 'error');
      setPlaceholder('This session is already open in another tab. Disconnect or terminate it before resuming here.', true);
      addActivity('warning', 'Session already attached', id);
      return;
    }
    var terminalSessionId = meta.source === 'history' ? makeAutoSessionId() : id;
    var previousSession = sessionInput.value;
    sessionInput.value = terminalSessionId;
    autoSession = terminalSessionId !== mainSessionId && terminalSessionId.indexOf('web-') === 0;
    if (autoSession) window.sessionStorage.setItem(autoSessionKey, terminalSessionId);
    attachRetryCount = 0;
    syncSessionUrl(terminalSessionId);
    rememberSessionId(terminalSessionId);
    resetActivity();
    setPlaceholder(activeResumeSessionId ? 'Opening persisted session ' + id + '...' : 'Opening session ' + id + '...', true);
    if (ws) {
      ws.onclose = null;
      try { ws.close(1000); } catch(e) { /* already closed */ }
      ws = null;
    }
    connected = false;
    connecting = false;
    terminateBtn.disabled = true;
    connectBtn.textContent = 'Connect';
    addActivity('session', activeResumeSessionId ? 'Restoring persisted session' : 'Switching session', previousSession ? previousSession + ' -> ' + terminalSessionId : terminalSessionId);
    window.setTimeout(function() { connect({ retry: true, ensure: true }); }, 180);
  }

  function createDetachedAgentSession() {
    return resolveCaptainAgentId().then(function(agentId) {
      return fetch('/api/agents/' + encodeURIComponent(agentId) + '/sessions', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ activate: false })
      });
    }).then(function(response) {
      return response.json().catch(function() { return {}; }).then(function(payload) {
        if (!response.ok) {
          throw new Error(payload && (payload.error || payload.message) || 'session creation failed');
        }
        var persistedId = payload && payload.session_id || '';
        if (!validUuid(persistedId)) throw new Error('daemon returned no valid session ID');
        return persistedId;
      });
    });
  }

  function createNewSession() {
    if (newSessionPending) return;
    newSessionPending = true;
    var previousSession = sessionInput.value;
    if (newSessionBtn) newSessionBtn.disabled = true;
    setStatus('creating persisted chat', 'idle');
    setPlaceholder('Creating a durable Captain session...', true);
    createDetachedAgentSession().then(function(persistedId) {
      activeResumeSessionId = null;
      var nextSession = rotateAutoSessionId();
      activeResumeSessionId = persistedId;
      syncSessionUrl(nextSession);
      attachRetryCount = 0;
      rememberSessionId(nextSession);
      resetActivity();
      term.reset();
      setPlaceholder('Opening new Captain chat...', true);
      if (ws) {
        ws.onclose = null;
        try { ws.close(1000); } catch(e) { /* already closed */ }
        ws = null;
      }
      connected = false;
      connecting = false;
      terminateBtn.disabled = true;
      connectBtn.textContent = 'Connect';
      addActivity('session', 'New persisted session', persistedId);
      loadTerminalSessions();
      window.setTimeout(function() { connect({ retry: true, ensure: true }); }, 180);
    }).catch(function(error) {
      setStatus('session creation failed', 'error');
      setPlaceholder('Captain could not create a durable session. Retry after checking daemon status.', true);
      addActivity('error', 'Session creation failed', error && error.message || String(error));
    }).then(function() {
      newSessionPending = false;
      if (newSessionBtn) newSessionBtn.disabled = false;
    });
  }

  function terminateTerminalSession(id, modeName) {
    if (!validSessionId(id)) return;
    var modeParam = (modeName || mode || 'captain').trim() || 'captain';
    addActivity('warning', 'Terminating session', id);
    fetch('/api/terminal/sessions/' + encodeURIComponent(id) + '?mode=' + encodeURIComponent(modeParam), {
      method: 'DELETE',
      credentials: 'same-origin'
    }).then(function(r) {
      return r.json().catch(function() { return {}; }).then(function(payload) {
        if (!r.ok) throw new Error(payload && (payload.error || payload.message) || 'terminate failed');
        return payload;
      });
    }).then(function() {
      addActivity('success', 'Session terminated', id);
      forgetSessionId(id);
      if (id === sessionInput.value) {
        if (ws) {
          ws.onclose = null;
          try { ws.close(1000); } catch(e) { /* already closed */ }
          ws = null;
        }
        connected = false;
        connecting = false;
        terminateBtn.disabled = true;
        connectBtn.textContent = 'Connect';
        setStatus('session terminated:' + id, 'idle');
        setPlaceholder('Session terminated.', true);
      }
      loadTerminalSessions();
    }).catch(function(error) {
      addActivity('error', 'Terminate failed', error && error.message ? error.message : id);
      setStatus('terminate failed:' + id, 'error');
    });
  }

  function fitAndResize() {
    applyTerminalDensity();
    if (fitAddon) {
      try { fitAddon.fit(); } catch(e) { /* layout not ready */ }
    }
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
    }
  }

  function syncViewportHeight() {
    var height = window.visualViewport ? window.visualViewport.height : window.innerHeight;
    if (height) document.documentElement.style.setProperty('--terminal-vh', height + 'px');
  }

  function applyTerminalDensity() {
    var width = window.innerWidth || 0;
    var height = window.innerHeight || 0;
    var next = 13;
    if (width <= 420) {
      next = 10;
    } else if (width <= 900 || height <= 520) {
      next = 11;
    }
    if (terminalFontSize === next) return;
    terminalFontSize = next;
    term.options.fontSize = next;
    term.options.lineHeight = next <= 10 ? 1.08 : 1.15;
  }

  function disconnect() {
    if (ws) {
      ws.onclose = null;
      ws.close(1000);
      ws = null;
    }
    connected = false;
    connecting = false;
    terminateBtn.disabled = true;
    connectBtn.textContent = 'Connect';
    setStatus('disconnected', 'idle');
    setPlaceholder('Disconnected.', true);
    addActivity('warning', 'Disconnected', 'Browser detached from ' + sessionId());
  }

  function retryWithFreshAutoSession(message) {
    if (!autoSession || attachRetryCount >= 2 || !/attached browser/i.test(message || '')) {
      return false;
    }
    attachRetryCount += 1;
    rotateAutoSessionId();
    if (ws) {
      ws.onclose = null;
      try { ws.close(1000); } catch(e) { /* already closed */ }
      ws = null;
    }
    connected = false;
    connecting = false;
    terminateBtn.disabled = true;
    connectBtn.textContent = 'Connect';
    setStatus('opening fresh chat:' + sessionId(), 'idle');
    setPlaceholder('Previous web chat is still attached. Opening a fresh chat session...', true);
    window.setTimeout(function() { connect({ retry: true, ensure: true }); }, 250);
    return true;
  }

  function checkAccess() {
    return fetch('/api/auth/check', { credentials: 'same-origin' })
      .then(function(r) { return r.ok ? r.json() : { mode: 'unknown' }; })
      .then(function(info) {
        if (info && info.mode === 'session') {
          if (info.authenticated) {
            hideAuthPanel();
            setPlaceholder('Opening Captain chat...', true);
            return true;
          }
          showAuthPanel('session', 'Sign in with your Captain web credentials to open the terminal.');
          setStatus('authentication required', 'error');
          return false;
        }
        if (info && info.mode === 'apikey') {
          showAuthPanel('none', 'Web terminal login is not configured. Run setup or ask Captain to create web credentials.');
          setStatus('web auth not configured', 'error');
          return false;
        }
        if (!info || info.mode === 'none') {
          showAuthPanel('none', 'Web terminal login is not configured. Run setup or ask Captain to create web credentials.');
          setStatus('terminal auth not configured', 'error');
          return false;
        }
        if (info.mode === 'unknown') {
          showAuthPanel('none', 'Cannot verify web authentication state. Check the daemon logs.');
          setStatus('auth check failed', 'error');
          return false;
        }
        hideAuthPanel();
        return true;
      })
      .catch(function() {
        showAuthPanel('none', 'Cannot verify web authentication state. Check the daemon logs.');
        setStatus('auth check failed', 'error');
        return false;
      });
  }

  function connect() {
    var options = arguments[0] || {};
    if (connected) {
      if (options.ensure) return;
      disconnect();
      return;
    }
    if (connecting) return;
    ensureInitialSessionId();
    if (!options.retry) attachRetryCount = 0;
    fitAndResize();
    setStatus((activeResumeSessionId ? 'restoring chat:' : 'connecting chat:') + sessionId(), 'idle');
    setPlaceholder(activeResumeSessionId ? 'Restoring Captain session...' : 'Connecting to Captain chat...', true);
    addActivity('session', activeResumeSessionId ? 'Restoring' : 'Connecting', sessionId());
    connecting = true;
    ws = new WebSocket(terminalUrl());

    ws.onopen = function() {
      connecting = false;
      connected = true;
      connectBtn.textContent = 'Disconnect';
      terminateBtn.disabled = false;
      setStatus((activeResumeSessionId ? 'restored chat:' : 'connected chat:') + sessionId(), 'connected');
      if (replayChip) replayChip.textContent = 'live replay';
      term.reset();
      resetTerminalInputLine();
      setPlaceholder(activeResumeSessionId ? 'Loading persisted history...' : 'Starting Captain chat...', true);
      addActivity('success', activeResumeSessionId ? 'Persisted session attached' : 'Connected', sessionId());
      loadTerminalSessions();
      fitAndResize();
      term.focus();
    };

    ws.onmessage = function(event) {
      var msg;
      try {
        msg = JSON.parse(event.data);
      } catch(e) {
        return;
      }
      if (msg.type === 'output') {
        setPlaceholder('', false);
        handleOutput(msg.data || '');
        term.write(msg.data || '');
      } else if (msg.type === 'error') {
        if (retryWithFreshAutoSession(msg.message || '')) return;
        setStatus(msg.message || 'terminal error', 'error');
        setPlaceholder(msg.message || 'Terminal error.', true);
        addActivity('error', 'Terminal error', msg.message || 'terminal error');
        term.writeln('\r\n\x1b[31m' + (msg.message || 'terminal error') + '\x1b[0m');
      } else if (msg.type === 'exit') {
        connected = false;
        terminateBtn.disabled = true;
        connectBtn.textContent = 'Connect';
        setStatus('process exited ' + (msg.code === null ? '' : msg.code), 'idle');
        setPlaceholder('Captain chat process exited.', true);
        addActivity('warning', 'Process exited', msg.code === null ? 'No exit code' : String(msg.code));
      }
    };

    ws.onerror = function() {
      connecting = false;
      setStatus('connection error', 'error');
      setPlaceholder('Connection error.', true);
      addActivity('error', 'Connection error', sessionId());
    };

    ws.onclose = function() {
      connecting = false;
      connected = false;
      ws = null;
      terminateBtn.disabled = true;
      connectBtn.textContent = 'Connect';
      setStatus('disconnected', 'idle');
      setPlaceholder('Disconnected.', true);
      loadTerminalSessions();
    };
  }

  function classifyLine(line) {
    var clean = stripAnsi(line).replace(/\s+/g, ' ').trim();
    var text = clean.toLowerCase();
    if (!clean) return null;
    if (/^[\s─━═│┃┌┐└┘├┤┬┴┼╔╗╚╝╠╣╦╩╬╭╮╰╯╥╨╫╪▁▂▃▄▅▆▇█]+$/.test(clean)) return null;
    if (/\/help for commands|enter envoyer|alt\+enter|ctrl\+m|unleash the future|ctx .* tok/.test(text)) return null;
    if (/^[a-z0-9._/-]+\/[a-z0-9._-]+\s+│\s+daemon/.test(text)) return null;
    if (/error|failed|erreur|panic|denied|unauthorized|exception/.test(text)) return 'error';
    if (/browser_|browser batch|screenshot|navigate|click|web_fetch|web_search|web_download|web_research|browser action/.test(text)) return 'browser';
    if (/ssh_exec|shell_exec|config_read|captain_docs|capability_search|memory_save|document_|tool|▸|✔/.test(text)) return /done|✔|success/.test(text) ? 'success' : 'tool';
    if (/model_switch|default_model|switché|switched|provider:|model:/.test(text)) return 'model';
    if (/approval|confirm|warning|attente|toujours en cours|pending/.test(text)) return 'warning';
    return null;
  }

  function handleOutput(data) {
    stripAnsi(data).split(/\r?\n/).forEach(function(line) {
      var kind = classifyLine(line);
      if (!kind) return;
      var title = kind === 'browser' ? 'Browser activity'
        : kind === 'tool' ? 'Tool call'
        : kind === 'success' ? 'Tool done'
        : kind === 'model' ? 'Model signal'
        : kind === 'warning' ? 'Attention'
        : 'Error';
      addActivity(kind, title, stripAnsi(line).replace(/\s+/g, ' ').trim());
      if (kind === 'browser') updateBrowserPreview(line);
    });
  }

  function addActivity(kind, title, detail) {
    var now = Date.now();
    var key = [kind || 'tool', title || 'Activity', detail || ''].join('\u0000');
    if (key === lastActivityKey && now - lastActivityAt < 2500) return;
    lastActivityKey = key;
    lastActivityAt = now;
    var entry = {
      kind: kind || 'tool',
      title: title || 'Activity',
      detail: detail || '',
      expanded: false,
      at: new Date()
    };
    activity.unshift(entry);
    if (activity.length > maxActivity) activity.length = maxActivity;
    renderActivity();
  }

  function renderActivity() {
    if (!activityList) return;
    if (!activity.length) {
      activityList.innerHTML = '<div class="terminal-empty">Waiting for activity</div>';
      if (activityCount) activityCount.textContent = '0';
      return;
    }
    if (activityCount) activityCount.textContent = String(activity.length);
    activityList.innerHTML = activity.map(function(item, index) {
      var expandable = (item.detail || '').length > 130;
      var detail = item.expanded ? item.detail : shortText(item.detail, 130);
      return [
        '<button type="button" class="terminal-activity-card kind-' + escapeHtml(item.kind) + (expandable ? ' expandable' : '') + '" data-activity-index="' + index + '" aria-expanded="' + (item.expanded ? 'true' : 'false') + '">',
        '<div class="terminal-activity-meta"><span class="terminal-activity-dot"></span><span>' + item.at.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) + '</span><span>' + escapeHtml(item.kind) + '</span></div>',
        '<div class="terminal-activity-title">' + escapeHtml(item.title) + '</div>',
        '<div class="terminal-activity-detail">' + escapeHtml(detail) + '</div>',
        '</button>'
      ].join('');
    }).join('');
  }

  function sessionEventPayload(event) {
    return event && event.payload && typeof event.payload === 'object' ? event.payload : {};
  }

  function showSessionEvent(event) {
    if (!event || sessionEventSeen[event.id]) return;
    var payload = sessionEventPayload(event);
    var phase = String(payload.phase || '');
    var isCompaction = event.event_type === 'compaction'
      || (event.event_type === 'phase_change' && /^compact/.test(phase));
    if (!isCompaction) return;

    sessionEventSeen[event.id] = true;
    var detail = shortText(payload.detail || phase || 'Context compaction', 180);
    var kind = phase === 'compaction_failed' ? 'error'
      : phase === 'compacted' ? 'success'
      : 'warning';
    var title = phase === 'compacted' ? 'Context compacted'
      : phase === 'compaction_failed' ? 'Compaction failed'
      : 'Context compaction';
    addActivity(kind, title, detail);
    if (phase === 'compacting') {
      setStatus('context compaction running', 'idle');
    } else if (phase === 'compacted') {
      setStatus('context compacted', connected ? 'connected' : 'idle');
    } else if (phase === 'compaction_failed') {
      setStatus('context compaction failed', 'error');
    }
  }

  function pollSessionEvents() {
    resolveCaptainAgentId()
      .then(function(agentId) {
        var from = Math.max(0, sessionEventCursor);
        return fetch('/api/sessions/' + encodeURIComponent(agentId) + '/events?from=' + encodeURIComponent(String(from)) + '&limit=80', {
          credentials: 'same-origin'
        });
      })
      .then(function(r) { return r.ok ? r.json() : null; })
      .then(function(payload) {
        var events = payload && Array.isArray(payload.events) ? payload.events : [];
        var newest = sessionEventCursor;
        events.forEach(function(event) {
          if (typeof event.ts === 'number') newest = Math.max(newest, event.ts + 1);
          showSessionEvent(event);
        });
        sessionEventCursor = Math.max(newest, Date.now() - 30000);
      })
      .catch(function() {});
  }

  function startSessionEventPolling() {
    if (sessionEventPollTimer) return;
    pollSessionEvents();
    sessionEventPollTimer = window.setInterval(pollSessionEvents, 5000);
  }

  function updateMetrics() {
    return fetch('/api/usage/summary', { credentials: 'same-origin' })
      .then(function(r) { return r.ok ? r.json() : null; })
      .then(function(summary) {
        if (!summary) return;
        var input = Number(summary.total_input_tokens || 0);
        var output = Number(summary.total_output_tokens || 0);
        if (metricTokens) metricTokens.textContent = formatCompactNumber(input + output);
        if (metricCost) metricCost.textContent = formatCost(summary.total_cost_usd || 0);
        if (metricCalls) metricCalls.textContent = formatCompactNumber(summary.call_count || 0);
      })
      .catch(function() {
        if (metricTokens) metricTokens.textContent = 'n/a';
      });
  }

  function updateBrowserPreview(line) {
    if (!browserPreview || !browserPreviewText) return;
    browserPreview.hidden = false;
    browserPreviewText.textContent = shortText(line, 110) || 'Browser activity detected';
  }

  function sendCommand(value) {
    var command = (value || '').trim();
    if (!command) return;
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      setPlaceholder('Connect before sending commands.', true);
      addActivity('warning', 'Command not sent', 'Terminal is not connected');
      return;
    }
    ws.send(JSON.stringify({ type: 'input', data: command + '\r' }));
    rememberTerminalInput(command + '\r');
    addActivity('model', 'Command sent', command);
    if (commandInput) commandInput.value = '';
    term.focus();
  }

  function sendTerminalInput(data) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'input', data: data }));
      rememberTerminalInput(data);
      return true;
    }
    return false;
  }

  function setCallUiState(state, label) {
    if (callToggleBtn) {
      callToggleBtn.disabled = false;
      callToggleBtn.textContent = label || (state === 'listening' ? 'Release' : state === 'active' ? 'Hold' : state === 'connecting' ? 'Wait' : 'Hold');
      callToggleBtn.classList.toggle('call-active', state === 'active' || state === 'listening');
      callToggleBtn.classList.toggle('call-connecting', state === 'connecting');
      callToggleBtn.classList.toggle('call-listening', state === 'listening');
      callToggleBtn.setAttribute('aria-pressed', state === 'listening' ? 'true' : 'false');
    }
    if (callEndBtn) {
      callEndBtn.hidden = !(state === 'active' || state === 'listening' || state === 'connecting');
    }
    if (callChip) {
      callChip.textContent = state === 'listening' ? 'mic live'
        : state === 'active' ? 'call ready'
        : state === 'connecting' ? 'calling'
        : 'chat + call';
    }
  }

  function setCallStatus(message, kind) {
    if (!callStatus) return;
    var text = (message || '').trim();
    callStatus.hidden = !text;
    callStatus.textContent = text;
    callStatus.classList.toggle('error', kind === 'error');
    callStatus.classList.toggle('warning', kind === 'warning');
  }

  function secondsToMs(value, fallbackMs) {
    var n = Number(value);
    if (!Number.isFinite(n) || n < 0) return fallbackMs;
    return Math.round(n * 1000);
  }

  function applyLiveCallConfig(config) {
    if (!config || typeof config !== 'object') return;
    callAutoEndSilenceMs = secondsToMs(config.auto_end_silence_secs, callAutoEndSilenceMs);
    callAutoEndInactiveMs = secondsToMs(config.auto_end_inactive_secs, callAutoEndInactiveMs);
    if (callChip && config.model) {
      callChip.title = 'Realtime model: ' + config.model;
    }
  }

  function loadLiveCallConfig() {
    return fetch('/api/realtime/calls', {
      method: 'GET',
      credentials: 'same-origin'
    }).then(function(response) {
      if (!response.ok) return null;
      return response.json();
    }).then(function(config) {
      applyLiveCallConfig(config);
    }).catch(function() {
      addActivity('warning', 'Call config unavailable', 'Using default safety limits');
    });
  }

  function markCallActivity(kind) {
    var now = Date.now();
    if (kind === 'sound') callLastSoundAt = now;
    if (kind === 'realtime') callLastRealtimeAt = now;
  }

  function stopCallWatchdog() {
    if (callWatchdogTimer) {
      window.clearInterval(callWatchdogTimer);
      callWatchdogTimer = 0;
    }
  }

  function checkCallWatchdog() {
    if (!callActive && !callConnecting) return;
    var now = Date.now();
    if (callAutoEndSilenceMs > 0 && callLastSoundAt > 0 && now - callLastSoundAt > callAutoEndSilenceMs) {
      stopLiveCall('Auto-ended after microphone silence');
      return;
    }
    var lastDiscussionAt = Math.max(callLastSoundAt || 0, callLastRealtimeAt || 0);
    if (callAutoEndInactiveMs > 0 && lastDiscussionAt > 0 && now - lastDiscussionAt > callAutoEndInactiveMs) {
      stopLiveCall('Auto-ended after call inactivity');
    }
  }

  function startCallWatchdog() {
    stopCallWatchdog();
    var now = Date.now();
    callLastSoundAt = now;
    callLastRealtimeAt = now;
    callWatchdogTimer = window.setInterval(checkCallWatchdog, 1000);
  }

  function closeLiveCallParts() {
    stopCallWatchdog();
    stopMicSpectrum();
    callMicEnabled = false;
    if (callDataChannel) {
      try { callDataChannel.close(); } catch(e) { /* already closed */ }
      callDataChannel = null;
    }
    if (callPc) {
      try { callPc.close(); } catch(e) { /* already closed */ }
      callPc = null;
    }
    if (callMediaStream) {
      callMediaStream.getTracks().forEach(function(track) {
        try { track.stop(); } catch(e) { /* already stopped */ }
      });
      callMediaStream = null;
    }
    if (callAudioEl) {
      try { callAudioEl.srcObject = null; } catch(e) { /* ignore */ }
      if (callAudioEl.parentNode) callAudioEl.parentNode.removeChild(callAudioEl);
      callAudioEl = null;
    }
  }

  function clearMicSpectrumCanvas() {
    if (!callSpectrumCanvas) return;
    var ctx = callSpectrumCanvas.getContext('2d');
    if (ctx) ctx.clearRect(0, 0, callSpectrumCanvas.width, callSpectrumCanvas.height);
  }

  function stopMicSpectrum() {
    if (callSpectrumFrame) {
      window.cancelAnimationFrame(callSpectrumFrame);
      callSpectrumFrame = 0;
    }
    callSpectrumData = null;
    callTimeData = null;
    if (callMeter) callMeter.hidden = true;
    clearMicSpectrumCanvas();
    if (callMicSource) {
      try { callMicSource.disconnect(); } catch(e) { /* already disconnected */ }
      callMicSource = null;
    }
    if (callAudioContext) {
      try { callAudioContext.close(); } catch(e) { /* already closed */ }
      callAudioContext = null;
    }
    callAnalyser = null;
  }

  function resizeSpectrumCanvas() {
    if (!callSpectrumCanvas) return null;
    var rect = callSpectrumCanvas.getBoundingClientRect();
    var dpr = window.devicePixelRatio || 1;
    var width = Math.max(1, Math.round((rect.width || 118) * dpr));
    var height = Math.max(1, Math.round((rect.height || 16) * dpr));
    if (callSpectrumCanvas.width !== width) callSpectrumCanvas.width = width;
    if (callSpectrumCanvas.height !== height) callSpectrumCanvas.height = height;
    return { width: width, height: height, dpr: dpr };
  }

  function drawMicSpectrum() {
    if (!callAnalyser || !callSpectrumCanvas || !callSpectrumData) return;
    var size = resizeSpectrumCanvas();
    if (!size) return;
    var ctx = callSpectrumCanvas.getContext('2d');
    if (!ctx) return;

    if (!callMicEnabled) {
      clearMicSpectrumCanvas();
      callSpectrumFrame = window.requestAnimationFrame(drawMicSpectrum);
      return;
    }

    if (callTimeData) {
      callAnalyser.getByteTimeDomainData(callTimeData);
      var sum = 0;
      for (var t = 0; t < callTimeData.length; t++) {
        var centered = (callTimeData[t] - 128) / 128;
        sum += centered * centered;
      }
      if (Math.sqrt(sum / callTimeData.length) > callSoundThreshold) {
        markCallActivity('sound');
      }
    }

    callAnalyser.getByteFrequencyData(callSpectrumData);
    ctx.clearRect(0, 0, size.width, size.height);

    var bars = 28;
    var gap = Math.max(1, Math.round(size.dpr));
    var barWidth = Math.max(1, Math.floor((size.width - gap * (bars - 1)) / bars));
    var usableHeight = size.height - Math.max(2, Math.round(size.dpr * 2));
    for (var i = 0; i < bars; i++) {
      var start = Math.floor(i * callSpectrumData.length / bars);
      var end = Math.max(start + 1, Math.floor((i + 1) * callSpectrumData.length / bars));
      var peak = 0;
      for (var j = start; j < end; j++) {
        if (callSpectrumData[j] > peak) peak = callSpectrumData[j];
      }
      var level = peak / 255;
      var height = Math.round(usableHeight * level);
      var x = i * (barWidth + gap);
      var y = Math.round((size.height - height) / 2);
      var hue = 132 + Math.round(level * 54);
      ctx.fillStyle = 'hsl(' + hue + ' 84% ' + (48 + Math.round(level * 24)) + '%)';
      ctx.fillRect(x, y, barWidth, Math.max(1, height));
    }
    callSpectrumFrame = window.requestAnimationFrame(drawMicSpectrum);
  }

  function startMicSpectrum(stream) {
    stopMicSpectrum();
    if (!stream || !callSpectrumCanvas || !callMeter) return;
    var AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!AudioCtx) return;
    try {
      callAudioContext = new AudioCtx();
      callAnalyser = callAudioContext.createAnalyser();
      callAnalyser.fftSize = 128;
      callAnalyser.smoothingTimeConstant = 0.68;
      callMicSource = callAudioContext.createMediaStreamSource(stream);
      callMicSource.connect(callAnalyser);
      callSpectrumData = new Uint8Array(callAnalyser.frequencyBinCount);
      callTimeData = new Uint8Array(callAnalyser.fftSize);
      callMeter.hidden = !callMicEnabled;
      if (callAudioContext.state === 'suspended') {
        callAudioContext.resume().catch(function() {});
      }
      drawMicSpectrum();
    } catch(e) {
      stopMicSpectrum();
      addActivity('warning', 'Mic spectrum unavailable', e && e.message ? e.message : 'AudioContext failed');
    }
  }

  function setCallMicEnabled(enabled) {
    callMicEnabled = !!enabled;
    if (callMediaStream) {
      callMediaStream.getAudioTracks().forEach(function(track) {
        track.enabled = callMicEnabled;
      });
    }
    if (callMeter) callMeter.hidden = !callMicEnabled || (!callActive && !callConnecting);
    if (callMicEnabled) {
      markCallActivity('sound');
      if (callActive) {
        setCallUiState('listening', 'Release');
        setCallStatus('listening', '');
      }
    } else {
      clearMicSpectrumCanvas();
      if (callActive) {
        setCallUiState('active', 'Hold');
        setCallStatus('mic muted', '');
      }
    }
  }

  function stopLiveCall(reason) {
    var wasActive = callActive || callConnecting;
    callConnecting = false;
    callActive = false;
    callPushToTalkDown = false;
    callPointerId = null;
    closeLiveCallParts();
    setCallUiState('idle', 'Hold');
    if (reason && reason !== 'User ended call') {
      setCallStatus(reason, 'warning');
    } else {
      setCallStatus('', '');
    }
    if (wasActive) {
      var ended = reason || 'Live voice disconnected';
      addActivity('call', 'Call ended', ended);
      mirrorVoiceToTerminal('status', 'Call ended: ' + ended, { max: 240 });
    }
  }

  function sendRealtimeEvent(event) {
    if (!callDataChannel || callDataChannel.readyState !== 'open') return false;
    callDataChannel.send(JSON.stringify(event));
    return true;
  }

  function sendRealtimeToolOutput(callId, output) {
    if (!callId) return;
    sendRealtimeEvent({
      type: 'conversation.item.create',
      item: {
        type: 'function_call_output',
        call_id: callId,
        output: output || ''
      }
    });
    sendRealtimeEvent({
      type: 'response.create',
      response: {
        tool_choice: 'none',
        instructions: "Read Captain's tool output to the user as Captain. Do not add unrelated information. Keep it natural, concise, and in the user's language."
      }
    });
  }

  function parseCaptainToolCall(event) {
    if (!event || !event.type) return null;
    var name = event.name || '';
    if (event.type === 'response.function_call_arguments.done' && (name === 'captain_message' || name === 'captain_activity_summary')) {
      return { name: name, call_id: event.call_id, arguments: event.arguments || '{}' };
    }
    if (event.type === 'response.output_item.done' && event.item && event.item.type === 'function_call') {
      name = event.item.name || '';
      if (name === 'captain_message' || name === 'captain_activity_summary') {
        return { name: name, call_id: event.item.call_id || event.item.id, arguments: event.item.arguments || '{}' };
      }
    }
    return null;
  }

  function callCaptainFromVoice(message, callId) {
    var sequence = ++voiceMirrorSeq;
    mirrorVoiceToTerminal('voice', message, { sequence: sequence });
    mirrorVoiceToTerminal('status', 'Captain is executing the voice request', {
      sequence: sequence,
      pending: true,
      max: 240
    });
    return resolveCaptainAgentId().then(function(agentId) {
      addActivity('call', 'Captain task', shortText(message, 150));
      return fetch('/api/agents/' + encodeURIComponent(agentId) + '/message', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          message: message,
          channel_type: 'web_call',
          sender_name: 'Captain Call'
        })
      });
    }).then(function(r) {
      return r.json().then(function(payload) {
        if (!r.ok) throw new Error(payload && payload.error || 'Captain task failed');
        return payload && payload.response ? payload.response : 'Captain completed the request.';
      });
    }).then(function(output) {
      addActivity('success', 'Captain answered', shortText(output, 150));
      mirrorVoiceToTerminal('captain', output, { sequence: sequence });
      sendRealtimeToolOutput(callId, output);
    }).catch(function(error) {
      var message = error && error.message ? error.message : 'Captain task failed';
      addActivity('error', 'Captain task failed', message);
      mirrorVoiceToTerminal('error', message, { sequence: sequence });
      sendRealtimeToolOutput(callId, message);
    });
  }

  function handleRealtimeEvent(raw) {
    var event;
    try {
      event = JSON.parse(raw);
    } catch(e) {
      return;
    }
    markCallActivity('realtime');
    var toolCall = parseCaptainToolCall(event);
    if (toolCall && toolCall.call_id && !handledCallToolIds[toolCall.call_id]) {
      handledCallToolIds[toolCall.call_id] = true;
      var args = {};
      try { args = JSON.parse(toolCall.arguments || '{}'); } catch(e) { args = {}; }
      if (toolCall.name === 'captain_activity_summary') {
        var summary = summarizeVoiceEvents(args.limit);
        mirrorVoiceToTerminal('status', 'Captain activity summary requested', { max: 240 });
        sendRealtimeToolOutput(toolCall.call_id, summary);
      } else if (args.message) {
        callCaptainFromVoice(String(args.message), toolCall.call_id);
      }
      return;
    }
    if (event.type === 'session.created' || event.type === 'session.updated') {
      addActivity('call', 'Call ready', event.type);
      setCallStatus('call ready', '');
    } else if (event.type === 'input_audio_buffer.speech_started') {
      addActivity('call', 'Listening', 'Speech detected');
      setCallStatus('listening', '');
    } else if (event.type === 'response.audio_transcript.done' && event.transcript) {
      addActivity('call', 'Captain voice', shortText(event.transcript, 150));
      mirrorVoiceToTerminal('captain', event.transcript, { max: 1200 });
    } else if (event.type === 'error') {
      var err = event.error && (event.error.message || event.error.code) || 'Realtime error';
      addActivity('error', 'Call error', String(err));
      setCallStatus(String(err), 'error');
      mirrorVoiceToTerminal('error', String(err), { max: 1200 });
    }
  }

  function startLiveCall(options) {
    if (callConnecting || callActive) return;
    options = options || {};
    if (!window.RTCPeerConnection || !navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
      addActivity('error', 'Call unavailable', 'WebRTC or microphone access is unavailable');
      setCallStatus('micro/webRTC unavailable', 'error');
      return;
    }

    callConnecting = true;
    callPushToTalkDown = !!options.listen;
    handledCallToolIds = {};
    startCallWatchdog();
    setCallUiState('connecting', 'Wait');
    setCallStatus(callPushToTalkDown ? 'hold while connecting...' : 'microphone...', 'warning');
    addActivity('call', 'Calling Captain', 'Opening live WebRTC audio');
    mirrorVoiceToTerminal('status', 'Live voice call connecting. Hold the talk button to open the microphone.', { max: 240 });

    var pc = new RTCPeerConnection();
    callPc = pc;
    callDataChannel = pc.createDataChannel('oai-events');
    callDataChannel.addEventListener('open', function() {
      addActivity('call', 'Realtime channel', 'Connected');
    });
    callDataChannel.addEventListener('message', function(event) {
      handleRealtimeEvent(event.data);
    });

    pc.ontrack = function(event) {
      if (!callAudioEl) {
        callAudioEl = document.createElement('audio');
        callAudioEl.autoplay = true;
        callAudioEl.playsInline = true;
        callAudioEl.style.display = 'none';
        document.body.appendChild(callAudioEl);
      }
      callAudioEl.srcObject = event.streams[0];
      var play = callAudioEl.play();
      if (play && typeof play.catch === 'function') play.catch(function() {});
    };
    pc.onconnectionstatechange = function() {
      if (pc.connectionState === 'failed' || pc.connectionState === 'closed') {
        stopLiveCall(pc.connectionState);
      } else if (pc.connectionState === 'disconnected') {
        addActivity('warning', 'Call interrupted', 'WebRTC is trying to reconnect');
      }
    };

    navigator.mediaDevices.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true
      }
    }).then(function(stream) {
      callMediaStream = stream;
      setCallStatus('connecting realtime...', 'warning');
      setCallMicEnabled(callPushToTalkDown);
      startMicSpectrum(stream);
      stream.getTracks().forEach(function(track) {
        pc.addTrack(track, stream);
      });
      return pc.createOffer();
    }).then(function(offer) {
      return pc.setLocalDescription(offer).then(function() { return offer; });
    }).then(function(offer) {
      return fetch('/api/realtime/calls', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/sdp' },
        body: offer.sdp || ''
      });
    }).then(function(response) {
      return response.text().then(function(sdp) {
        if (!response.ok) throw new Error(sdp || 'Realtime setup failed');
        return pc.setRemoteDescription({ type: 'answer', sdp: sdp });
      });
    }).then(function() {
      callConnecting = false;
      callActive = true;
      setCallMicEnabled(callPushToTalkDown);
      if (callPushToTalkDown) {
        setCallUiState('listening', 'Release');
        setCallStatus('listening', '');
      } else {
        setCallUiState('active', 'Hold');
        setCallStatus('call ready, mic muted', '');
      }
      addActivity('call', 'Call live', 'Hold to talk, release to mute');
      mirrorVoiceToTerminal('status', 'Live voice call connected. Hold to talk; release to mute the microphone. The terminal remains available.', { max: 240 });
    }).catch(function(error) {
      var message = error && error.message ? error.message : 'Live call failed';
      stopLiveCall(message);
      setCallStatus(message, 'error');
      setStatus('call failed: ' + message, 'error');
      addActivity('error', 'Call failed', message);
    });
  }

  function beginPushToTalk(event) {
    if (event && event.pointerType && event.isPrimary === false) return;
    if (event && event.cancelable) event.preventDefault();
    if (callPushToTalkDown) return;
    callPushToTalkDown = true;
    if (event && event.pointerId !== undefined) {
      callPointerId = event.pointerId;
      if (callToggleBtn && callToggleBtn.setPointerCapture) {
        try { callToggleBtn.setPointerCapture(event.pointerId); } catch(e) { /* pointer already released */ }
      }
    }
    if (callActive) {
      setCallMicEnabled(true);
      addActivity('call', 'Push-to-talk', 'Microphone live');
      return;
    }
    if (callConnecting) {
      setCallMicEnabled(true);
      setCallStatus('hold while connecting...', 'warning');
      return;
    }
    startLiveCall({ listen: true });
  }

  function endPushToTalk(event) {
    if (event && event.pointerId !== undefined && callPointerId !== null && event.pointerId !== callPointerId) return;
    if (event && event.cancelable) event.preventDefault();
    if (event && event.pointerId !== undefined && callToggleBtn && callToggleBtn.releasePointerCapture) {
      try { callToggleBtn.releasePointerCapture(event.pointerId); } catch(e) { /* capture already released */ }
    }
    callPointerId = null;
    if (!callPushToTalkDown) return;
    callPushToTalkDown = false;
    setCallMicEnabled(false);
    if (callActive || callConnecting) {
      addActivity('call', 'Push-to-talk', 'Microphone muted');
    }
  }

  function cancelPushToTalk(event) {
    endPushToTalk(event);
  }

  function stopLiveCallFromUi() {
    stopLiveCall('User ended call');
    term.focus();
  }

  function isPlainTextInput(data) {
    return typeof data === 'string' && data.length > 0 && !/[\x00-\x1f\x7f\x1b]/.test(data);
  }

  function trimInputLineByChars(value, count) {
    var chars = Array.from(value || '');
    chars.splice(Math.max(0, chars.length - count), count);
    return chars.join('');
  }

  function resetTerminalInputLine() {
    terminalInputLine = '';
    terminalInputLineAt = Date.now();
  }

  function rememberTerminalInput(data) {
    if (typeof data !== 'string' || !data) return;
    var bracketStart = '\x1b[200~';
    var bracketEnd = '\x1b[201~';
    if (data.indexOf(bracketStart) === 0 && data.lastIndexOf(bracketEnd) === data.length - bracketEnd.length) {
      rememberTerminalInput(data.slice(bracketStart.length, data.length - bracketEnd.length));
      return;
    }
    if (data.indexOf('\x1b') !== -1) return;

    var chars = Array.from(data);
    for (var i = 0; i < chars.length; i++) {
      var ch = chars[i];
      if (ch === '\r' || ch === '\n' || ch === '\x03' || ch === '\x15') {
        terminalInputLine = '';
      } else if (ch === '\x7f' || ch === '\b') {
        terminalInputLine = trimInputLineByChars(terminalInputLine, 1);
      } else if (isPlainTextInput(ch)) {
        terminalInputLine += ch;
      }
    }
    terminalInputLineAt = Date.now();
  }

  function commonPrefixLength(a, b) {
    var max = Math.min(a.length, b.length);
    var i = 0;
    while (i < max && a.charCodeAt(i) === b.charCodeAt(i)) i += 1;
    return i;
  }

  function normalizeTerminalCompare(value) {
    return String(value || '')
      .normalize('NFKD')
      .replace(/[\u0300-\u036f]/g, '')
      .toLocaleLowerCase();
  }

  function commonPrefixChars(a, b, normalizer) {
    var ac = Array.from(a || '');
    var bc = Array.from(b || '');
    var max = Math.min(ac.length, bc.length);
    var i = 0;
    while (i < max) {
      var left = normalizer ? normalizer(ac[i]) : ac[i];
      var right = normalizer ? normalizer(bc[i]) : bc[i];
      if (left !== right) break;
      i += 1;
    }
    return i;
  }

  function sliceChars(value, start) {
    return Array.from(value || '').slice(start).join('');
  }

  function firstDifferentChar(a, b, limit) {
    var ac = Array.from(a || '');
    var bc = Array.from(b || '');
    var max = Math.min(limit, ac.length, bc.length);
    for (var i = 0; i < max; i++) {
      if (ac[i] !== bc[i]) return i;
    }
    return max;
  }

  function normalizeTextInputDelta(data) {
    var bracketStart = '\x1b[200~';
    var bracketEnd = '\x1b[201~';
    if (typeof data === 'string' && data.indexOf(bracketStart) === 0 && data.lastIndexOf(bracketEnd) === data.length - bracketEnd.length) {
      var pasted = data.slice(bracketStart.length, data.length - bracketEnd.length);
      var normalizedPaste = normalizeTextInputDelta(pasted);
      if (!normalizedPaste) return '';
      if (isPlainTextInput(normalizedPaste)) return bracketStart + normalizedPaste + bracketEnd;
      return data;
    }

    if (!isPlainTextInput(data)) return data;
    if (Array.from(data).length <= 1) return data;

    var previous = terminalInputLine || '';
    if (!previous) return data;

    if (data === previous) return '';
    if (data.indexOf(previous) === 0) return data.slice(previous.length);
    if (previous.indexOf(data) === 0) return '';

    var prefix = commonPrefixLength(previous, data);
    var similarPrefix = prefix >= 3 && prefix >= Math.floor(previous.length * 0.5);
    if (similarPrefix) {
      return data.slice(prefix);
    }

    var normalizedPrefix = commonPrefixChars(previous, data, normalizeTerminalCompare);
    var previousChars = Array.from(previous);
    var dataChars = Array.from(data);
    var similarNormalized = normalizedPrefix >= 3
      && normalizedPrefix >= Math.floor(Math.min(previousChars.length, dataChars.length) * 0.5);
    if (similarNormalized) {
      var changedAt = firstDifferentChar(previous, data, normalizedPrefix);
      if (changedAt < Math.min(previousChars.length, normalizedPrefix)) {
        var eraseCount = Math.max(0, previousChars.length - changedAt);
        return '\x7f'.repeat(eraseCount) + sliceChars(data, changedAt);
      }
      return sliceChars(data, normalizedPrefix);
    }

    var lowerPrefix = commonPrefixChars(previous, data, function(value) {
      return String(value || '').toLocaleLowerCase();
    });
    var similarCaseOnly = lowerPrefix >= 3 && lowerPrefix >= Math.floor(Math.min(previousChars.length, dataChars.length) * 0.5);
    if (similarCaseOnly) {
      return sliceChars(data, lowerPrefix);
    }

    return data;
  }

  function terminalWheelToPager(event) {
    if (!event || Math.abs(event.deltaY || 0) < 1) return true;
    if (event.ctrlKey || event.metaKey) return true;

    var steps = Math.max(1, Math.min(12, Math.ceil(Math.abs(event.deltaY) / 18)));
    term.scrollLines(event.deltaY < 0 ? -steps : steps);
    if (event.preventDefault) event.preventDefault();
    return false;
  }

  function sendPagerFromTouch(deltaY) {
    terminalTouchAccum += deltaY;
    if (Math.abs(terminalTouchAccum) < 18) return false;

    var steps = Math.max(1, Math.min(12, Math.floor(Math.abs(terminalTouchAccum) / 18)));
    terminalTouchAccum = terminalTouchAccum % 18;
    term.scrollLines(deltaY > 0 ? -steps : steps);
    return true;
  }

  function terminalTouchStart(event) {
    if (window.PointerEvent) return;
    if (!event || !event.touches || event.touches.length !== 1) {
      terminalTouchY = null;
      terminalTouchAccum = 0;
      return;
    }
    terminalTouchY = event.touches[0].clientY;
    terminalTouchAccum = 0;
  }

  function terminalTouchMove(event) {
    if (window.PointerEvent) return;
    if (terminalTouchY === null || !event || !event.touches || event.touches.length !== 1) return;
    var nextY = event.touches[0].clientY;
    var deltaY = nextY - terminalTouchY;
    terminalTouchY = nextY;
    if (Math.abs(deltaY) < 2) return;
    if (sendPagerFromTouch(deltaY)) {
      if (event.preventDefault) event.preventDefault();
      if (event.stopPropagation) event.stopPropagation();
    }
  }

  function terminalTouchEnd() {
    if (window.PointerEvent) return;
    terminalTouchY = null;
    terminalTouchAccum = 0;
  }

  function resetTerminalTouchState() {
    terminalTouchY = null;
    terminalTouchAccum = 0;
  }

  function terminalPointerStart(event) {
    if (!event || event.pointerType !== 'touch' || event.isPrimary === false) return;
    terminalPointerId = event.pointerId;
    terminalTouchY = event.clientY;
    terminalTouchAccum = 0;
    if (terminalEl.setPointerCapture) {
      try { terminalEl.setPointerCapture(event.pointerId); } catch(e) { /* capture unavailable */ }
    }
  }

  function terminalPointerMove(event) {
    if (!event || event.pointerType !== 'touch' || terminalPointerId !== event.pointerId || terminalTouchY === null) return;
    var deltaY = event.clientY - terminalTouchY;
    terminalTouchY = event.clientY;
    if (Math.abs(deltaY) < 2) return;
    if (sendPagerFromTouch(deltaY)) {
      if (event.preventDefault) event.preventDefault();
      if (event.stopPropagation) event.stopPropagation();
    }
  }

  function terminalPointerEnd(event) {
    if (event && terminalPointerId !== null && event.pointerId !== terminalPointerId) return;
    if (event && terminalEl.releasePointerCapture) {
      try { terminalEl.releasePointerCapture(event.pointerId); } catch(e) { /* capture already released */ }
    }
    terminalPointerId = null;
    resetTerminalTouchState();
  }

  function resolveCaptainAgentId() {
    if (cachedAgentId) return Promise.resolve(cachedAgentId);
    return fetch('/api/agents', { credentials: 'same-origin' })
      .then(function(r) {
        if (!r.ok) throw new Error('agents unavailable');
        return r.json();
      })
      .then(function(payload) {
        var agents = Array.isArray(payload)
          ? payload
          : (Array.isArray(payload && payload.agents) ? payload.agents : []);
        var captain = agents.find(function(agent) {
          return agent && (agent.name === 'captain' || agent.name === 'Captain');
        }) || agents[0];
        if (!captain || !captain.id) throw new Error('no agent available');
        cachedAgentId = captain.id;
        return cachedAgentId;
      });
  }

  function uploadAttachment(file) {
    return resolveCaptainAgentId().then(function(agentId) {
      addActivity('tool', 'Uploading file', file.name || 'attachment');
      return fetch('/api/agents/' + encodeURIComponent(agentId) + '/upload', {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'content-type': file.type || 'application/octet-stream',
          'x-filename': file.name || 'attachment'
        },
        body: file
      });
    }).then(function(r) {
      return r.json().then(function(payload) {
        if (!r.ok) throw new Error(payload && payload.error || 'upload failed');
        return payload;
      });
    }).then(function(payload) {
      var path = payload.local_path || payload.path || '';
      var name = payload.filename || file.name || 'attachment';
      addActivity('success', 'File ready', shortText(name + (path ? ' -> ' + path : ''), 150));
      if (payload.transcription) {
        sendCommand('Message vocal transcrit depuis ' + name + ':\n' + payload.transcription);
        return payload;
      }
      if (path) {
        var prefix = (payload.content_type || file.type || '').indexOf('image/') === 0 ? '/image ' : '/file ';
        sendCommand(prefix + path);
      } else if (payload.file_id) {
        sendCommand('Analyse cette pièce jointe Captain: /api/uploads/' + payload.file_id + ' (' + name + ')');
      }
      return payload;
    }).catch(function(error) {
      addActivity('error', 'Upload failed', error && error.message ? error.message : 'upload failed');
    });
  }

  function uploadAttachments(files) {
    var list = Array.prototype.slice.call(files || []);
    if (!list.length) return;
    list.reduce(function(chain, file) {
      return chain.then(function() { return uploadAttachment(file); });
    }, Promise.resolve());
  }

  function setPanelState() {
    if (!workbench) return;
    workbench.classList.toggle('sessions-collapsed', sessionsToggle && sessionsToggle.getAttribute('aria-expanded') !== 'true');
    workbench.classList.toggle('activity-collapsed', activityToggle && activityToggle.getAttribute('aria-expanded') !== 'true');
    window.requestAnimationFrame(fitAndResize);
  }

  function isCompactViewport() {
    return !!(window.matchMedia && window.matchMedia('(max-width: 900px), (max-height: 520px)').matches);
  }

  function applyResponsiveDefaults() {
    var compact = isCompactViewport();
    if (responsiveMode === compact) return;
    responsiveMode = compact;
    if (compact) {
      if (sessionsToggle && sessionsToggle.dataset.userToggled !== 'true') {
        sessionsToggle.setAttribute('aria-expanded', 'false');
      }
      if (activityToggle && activityToggle.dataset.userToggled !== 'true') {
        activityToggle.setAttribute('aria-expanded', 'false');
      }
      if (commandToggle && commandToggle.dataset.userToggled !== 'true') {
        commandToggle.setAttribute('aria-expanded', 'false');
        if (commandBar) commandBar.hidden = true;
      }
    } else {
      if (sessionsToggle && sessionsToggle.dataset.userToggled !== 'true') {
        sessionsToggle.setAttribute('aria-expanded', 'true');
      }
      if (activityToggle && activityToggle.dataset.userToggled !== 'true') {
        activityToggle.setAttribute('aria-expanded', 'true');
      }
    }
    setPanelState();
  }

  term.onData(function(data) {
    configureTerminalKeyboardInput();
    var normalized = normalizeTextInputDelta(data);
    if (normalized) sendTerminalInput(normalized);
    clearTerminalHelperInputSoon();
  });

  if (typeof term.attachCustomWheelEventHandler === 'function') {
    term.attachCustomWheelEventHandler(terminalWheelToPager);
  } else {
    terminalEl.addEventListener('wheel', terminalWheelToPager, { passive: false });
  }
  terminalEl.addEventListener('touchstart', terminalTouchStart, { passive: true, capture: true });
  terminalEl.addEventListener('touchmove', terminalTouchMove, { passive: false, capture: true });
  terminalEl.addEventListener('touchend', terminalTouchEnd, { passive: true, capture: true });
  terminalEl.addEventListener('touchcancel', terminalTouchEnd, { passive: true, capture: true });
  if (window.PointerEvent) {
    terminalEl.addEventListener('pointerdown', terminalPointerStart, { passive: true, capture: true });
    terminalEl.addEventListener('pointermove', terminalPointerMove, { passive: false, capture: true });
    terminalEl.addEventListener('pointerup', terminalPointerEnd, { passive: true, capture: true });
    terminalEl.addEventListener('pointercancel', terminalPointerEnd, { passive: true, capture: true });
  }

  term.onBinary(function(data) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    var bytes = new Uint8Array(data.length);
    for (var i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 255;
    ws.send(bytes);
  });

  term.onResize(function(size) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', rows: size.rows, cols: size.cols }));
    }
  });

  connectBtn.addEventListener('click', connect);
  if (newSessionBtn) {
    newSessionBtn.addEventListener('click', createNewSession);
  }
  if (sessionsToggle) {
    sessionsToggle.addEventListener('click', function() {
      sessionsToggle.dataset.userToggled = 'true';
      var expanded = sessionsToggle.getAttribute('aria-expanded') === 'true';
      sessionsToggle.setAttribute('aria-expanded', expanded ? 'false' : 'true');
      setPanelState();
    });
  }
  if (activityToggle) {
    activityToggle.addEventListener('click', function() {
      activityToggle.dataset.userToggled = 'true';
      var expanded = activityToggle.getAttribute('aria-expanded') === 'true';
      activityToggle.setAttribute('aria-expanded', expanded ? 'false' : 'true');
      setPanelState();
    });
  }
  if (commandToggle && commandBar) {
    commandToggle.addEventListener('click', function() {
      commandToggle.dataset.userToggled = 'true';
      var opening = commandBar.hidden;
      commandBar.hidden = !opening;
      commandToggle.setAttribute('aria-expanded', opening ? 'true' : 'false');
      if (opening && commandInput) window.requestAnimationFrame(function() { commandInput.focus(); });
      window.requestAnimationFrame(fitAndResize);
    });
  }
  if (attachmentButton && attachmentInput) {
    attachmentButton.addEventListener('click', function() {
      attachmentInput.click();
    });
    attachmentInput.addEventListener('change', function() {
      uploadAttachments(attachmentInput.files);
      attachmentInput.value = '';
    });
  }
  if (callToggleBtn) {
    if (window.PointerEvent) {
      callToggleBtn.addEventListener('pointerdown', beginPushToTalk);
      callToggleBtn.addEventListener('pointerup', endPushToTalk);
      callToggleBtn.addEventListener('pointercancel', cancelPushToTalk);
      callToggleBtn.addEventListener('lostpointercapture', function() {
        if (callPushToTalkDown) cancelPushToTalk();
      });
    } else {
      callToggleBtn.addEventListener('mousedown', beginPushToTalk);
      document.addEventListener('mouseup', endPushToTalk);
      callToggleBtn.addEventListener('touchstart', beginPushToTalk, { passive: false });
      document.addEventListener('touchend', endPushToTalk, { passive: false });
      document.addEventListener('touchcancel', cancelPushToTalk, { passive: false });
    }
    callToggleBtn.addEventListener('click', function(event) {
      event.preventDefault();
    });
    callToggleBtn.addEventListener('keydown', function(event) {
      if (event.key === ' ' || event.key === 'Enter') beginPushToTalk(event);
    });
    callToggleBtn.addEventListener('keyup', function(event) {
      if (event.key === ' ' || event.key === 'Enter') endPushToTalk(event);
    });
    callToggleBtn.addEventListener('blur', cancelPushToTalk);
    setCallUiState('idle', 'Hold');
    loadLiveCallConfig();
  }
  if (callEndBtn) {
    callEndBtn.addEventListener('click', stopLiveCallFromUi);
  }
  if (voiceTranscriptClear) {
    voiceTranscriptClear.addEventListener('click', function() {
      voiceEvents = [];
      renderVoiceTranscript();
      term.focus();
    });
  }
  if (refreshSessionsBtn) {
    refreshSessionsBtn.addEventListener('click', loadTerminalSessions);
  }
  if (sessionList) {
    sessionList.addEventListener('click', function(event) {
      var kill = event.target.closest('[data-terminate-session]');
      if (kill) {
        event.preventDefault();
        event.stopPropagation();
        terminateTerminalSession(
          kill.getAttribute('data-terminate-session'),
          kill.getAttribute('data-mode') || 'captain'
        );
        return;
      }
      var card = event.target.closest('[data-session]');
      if (!card) return;
      switchSession(card.getAttribute('data-session'), {
        active_clients: card.getAttribute('data-active-clients') || '0',
        replay_bytes: card.getAttribute('data-replay-bytes') || '0',
        local: card.getAttribute('data-local') === 'true',
        source: card.getAttribute('data-source') || '',
        mode: card.getAttribute('data-mode') || 'captain',
        restorable: card.getAttribute('data-restorable') !== 'false',
        resume_session: card.getAttribute('data-resume-session') || ''
      });
    });
  }
  if (activityList) {
    activityList.addEventListener('click', function(event) {
      var card = event.target.closest('[data-activity-index]');
      if (!card) return;
      var index = Number(card.getAttribute('data-activity-index'));
      if (!Number.isInteger(index) || !activity[index]) return;
      activity[index].expanded = !activity[index].expanded;
      renderActivity();
    });
  }
  if (commandInput) {
    commandInput.addEventListener('keydown', function(event) {
      if (event.key === 'Enter') {
        event.preventDefault();
        sendCommand(commandInput.value);
      } else if (event.key === 'Escape') {
        commandBar.hidden = true;
        if (commandToggle) commandToggle.setAttribute('aria-expanded', 'false');
        term.focus();
      }
    });
  }
  commandButtons.forEach(function(button) {
    button.addEventListener('click', function() {
      sendCommand(button.getAttribute('data-command') || '');
    });
  });
  sessionInput.addEventListener('input', function() {
    autoSession = false;
    activeResumeSessionId = null;
    var value = (sessionInput.value || '').trim();
    var persisted = knownSessionItems.find(function(item) {
      return item.id === value && item.source === 'history' && validUuid(item.resume_session || item.id);
    });
    if (persisted) activeResumeSessionId = persisted.resume_session || persisted.id;
    attachRetryCount = 0;
  });
  sessionInput.addEventListener('change', function() {
    var value = (sessionInput.value || '').trim();
    if (validSessionId(value)) {
      syncSessionUrl(value);
      rememberSessionId(value);
    }
  });
  terminateBtn.addEventListener('click', function() {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'terminate' }));
    }
  });
  if (authForm) {
    authForm.addEventListener('submit', function(event) {
      event.preventDefault();
      if (authMode !== 'session') return;
      var username = (usernameInput && usernameInput.value || '').trim();
      var password = passwordInput && passwordInput.value || '';
      if (!username || !password) return;
      setStatus('signing in', 'idle');
      fetch('/api/auth/login', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: username, password: password })
      })
        .then(function(r) {
          if (!r.ok) {
            var err = new Error('login failed');
            err.status = r.status;
            throw err;
          }
          return r.json();
        })
        .then(function() {
          hideAuthPanel();
          if (passwordInput) passwordInput.value = '';
          setStatus('chat ready', 'idle');
          startSessionEventPolling();
          loadTerminalSessions().then(function(items) {
            if (hasStaleAutoSession(items)) {
              createNewSession();
              return;
            }
            bindKnownPersistedSession(items);
            connect();
          });
        })
        .catch(function(e) {
          // Distinguish wrong credentials (401) from the daemon being
          // unreachable/erroring — the generic "login failed" message sent
          // users hunting for a typo when the real problem was the daemon
          // being down.
          var message = e && e.status === 401
            ? 'Identifiants refusés. Vérifie le nom d\'utilisateur et le mot de passe web configurés.'
            : 'Connexion au daemon impossible. Vérifie qu\'il est démarré, puis réessaie.';
          showAuthPanel('session', message);
          setStatus('login failed', 'error');
        });
    });
  }
  window.addEventListener('resize', function() {
    syncViewportHeight();
    applyResponsiveDefaults();
    window.requestAnimationFrame(fitAndResize);
  });
  if (window.visualViewport) {
    window.visualViewport.addEventListener('resize', function() {
      syncViewportHeight();
      applyResponsiveDefaults();
      window.requestAnimationFrame(fitAndResize);
    });
  }
  window.addEventListener('beforeunload', function() {
    closeLiveCallParts();
  });

  syncViewportHeight();
  ensureInitialSessionId();
  (function() {
    var project = new URLSearchParams(window.location.search).get('project');
    if (project) addActivity('project', 'Project context', project);
  })();
  applyResponsiveDefaults();
  renderActivity();
  updateMetrics();
  window.setInterval(updateMetrics, 10000);
  setPanelState();
  loadTerminalSessions();
  setPlaceholder('Checking access...', true);
  setStatus('checking access', 'idle');
  checkAccess().then(function(allowed) {
    if (!allowed) {
      fitAndResize();
      return;
    }
    startSessionEventPolling();
    loadTerminalSessions().then(function(items) {
      fitAndResize();
      if (hasStaleAutoSession(items)) {
        createNewSession();
        return;
      }
      bindKnownPersistedSession(items);
      connect();
    });
  });
})();
