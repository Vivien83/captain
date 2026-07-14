// Captain Config — authenticated config.toml editor
'use strict';

(function() {
  var authPanel = document.getElementById('auth-panel');
  var authForm = document.getElementById('auth-form');
  var authHelp = document.getElementById('auth-help');
  var sessionFields = document.getElementById('session-fields');
  var usernameInput = document.getElementById('auth-username');
  var passwordInput = document.getElementById('auth-password');
  var editor = document.getElementById('config-editor');
  var templateEl = document.getElementById('config-template');
  var pathEl = document.getElementById('config-path');
  var saveBtn = document.getElementById('save');
  var reloadBtn = document.getElementById('reload');
  var validateBtn = document.getElementById('validate');
  var insertTemplateBtn = document.getElementById('insert-template');
  var searchInput = document.getElementById('config-search');
  var sectionList = document.getElementById('section-list');
  var lineCount = document.getElementById('line-count');
  var charCount = document.getElementById('char-count');
  var dirtyPill = document.getElementById('dirty-pill');
  var statusDot = document.getElementById('status-dot');
  var statusText = document.getElementById('status-text');
  var original = '';
  var template = '';

  if (!editor) return;

  function setStatus(text, state) {
    statusText.textContent = text;
    statusDot.classList.toggle('ok', state === 'ok');
    statusDot.classList.toggle('error', state === 'error');
  }

  function showAuth(message) {
    authHelp.textContent = message || 'Sign in with your Captain web credentials.';
    sessionFields.hidden = false;
    authPanel.hidden = false;
    window.requestAnimationFrame(function() {
      if (usernameInput) usernameInput.focus();
    });
  }

  function showBlocked(message) {
    authHelp.textContent = message;
    sessionFields.hidden = true;
    authPanel.hidden = false;
  }

  function hideAuth() {
    authPanel.hidden = true;
  }

  function apiJson(url, options) {
    var opts = options || {};
    opts.credentials = 'same-origin';
    opts.headers = Object.assign({ 'content-type': 'application/json' }, opts.headers || {});
    return fetch(url, opts).then(function(r) {
      return r.text().then(function(text) {
        var data = text ? JSON.parse(text) : {};
        if (!r.ok) {
          throw new Error(data.error || data.message || ('HTTP ' + r.status));
        }
        return data;
      });
    });
  }

  function checkAccess() {
    return apiJson('/api/auth/check', { method: 'GET' })
      .then(function(info) {
        if (info && info.mode === 'session' && info.authenticated) {
          hideAuth();
          return true;
        }
        if (info && info.mode === 'apikey') {
          showBlocked('Web login is not configured. Run setup or ask Captain to create web credentials before editing config in the browser.');
          setStatus('web auth not configured', 'error');
          return false;
        }
        showAuth('Sign in with your Captain web credentials to edit config.');
        setStatus('authentication required', 'error');
        return false;
      })
      .catch(function() {
        showAuth('Cannot verify web authentication state. Check the daemon logs.');
        setStatus('auth check failed', 'error');
        return false;
      });
  }

  function updateStats() {
    var value = editor.value || '';
    var dirty = value !== original;
    lineCount.textContent = value ? String(value.split(/\r?\n/).length) : '0';
    charCount.textContent = String(value.length);
    dirtyPill.textContent = dirty ? 'unsaved' : 'clean';
    dirtyPill.classList.toggle('dirty', dirty);
    saveBtn.disabled = !dirty;
    renderSections();
  }

  function extractSections(text) {
    var sections = [{ name: 'root', line: 0 }];
    text.split(/\r?\n/).forEach(function(line, idx) {
      var match = line.match(/^\s*\[([^\]]+)\]\s*$/);
      if (match) sections.push({ name: match[1], line: idx });
    });
    return sections;
  }

  function renderSections() {
    var query = (searchInput.value || '').trim().toLowerCase();
    var sections = extractSections(editor.value || '').filter(function(section) {
      return !query || section.name.toLowerCase().indexOf(query) !== -1;
    });
    sectionList.innerHTML = '';
    sections.slice(0, 80).forEach(function(section) {
      var btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'config-section-button';
      btn.textContent = section.name;
      btn.addEventListener('click', function() {
        jumpToLine(section.line);
      });
      sectionList.appendChild(btn);
    });
  }

  function jumpToLine(line) {
    var lines = (editor.value || '').split(/\r?\n/);
    var pos = 0;
    for (var i = 0; i < line && i < lines.length; i++) {
      pos += lines[i].length + 1;
    }
    editor.focus();
    editor.setSelectionRange(pos, pos);
    var approxLineHeight = 18;
    editor.scrollTop = Math.max(0, line * approxLineHeight - editor.clientHeight / 3);
  }

  function loadConfig() {
    setStatus('loading config', 'idle');
    return Promise.all([
      apiJson('/api/config/raw', { method: 'GET' }),
      apiJson('/api/config/template', { method: 'GET' })
    ]).then(function(results) {
      var current = results[0];
      var tpl = results[1];
      original = current.content || '';
      template = tpl.content || '';
      editor.value = original;
      templateEl.textContent = template;
      pathEl.textContent = current.path || 'config.toml';
      updateStats();
      setStatus('config loaded', 'ok');
    }).catch(function(e) {
      setStatus(e.message || 'load failed', 'error');
    });
  }

  function validateConfig() {
    setStatus('validating', 'idle');
    return apiJson('/api/config/validate', {
      method: 'POST',
      body: JSON.stringify({ content: editor.value || '' })
    }).then(function() {
      setStatus('valid config.toml', 'ok');
      return true;
    }).catch(function(e) {
      setStatus(e.message || 'invalid config', 'error');
      return false;
    });
  }

  function saveConfig() {
    validateConfig().then(function(ok) {
      if (!ok) return;
      setStatus('saving config', 'idle');
      apiJson('/api/config/raw', {
        method: 'PUT',
        body: JSON.stringify({ content: editor.value || '' })
      }).then(function(saved) {
        original = editor.value || '';
        updateStats();
        setStatus('saved, reloading', 'ok');
        return apiJson('/api/config/reload', { method: 'POST', body: '{}' })
          .then(function(reload) {
            var suffix = reload.restart_required ? ' · restart required' : '';
            setStatus('saved · ' + (reload.status || 'reloaded') + suffix, reload.restart_required ? 'idle' : 'ok');
            if (saved.snapshot) pathEl.textContent = 'backup: ' + saved.snapshot;
          })
          .catch(function(e) {
            if (saved.snapshot) pathEl.textContent = 'backup: ' + saved.snapshot;
            setStatus('saved · reload needs fresh login: ' + (e.message || 'auth changed'), 'error');
          });
      }).catch(function(e) {
        setStatus(e.message || 'save failed', 'error');
      });
    });
  }

  if (authForm) {
    authForm.addEventListener('submit', function(event) {
      event.preventDefault();
      var username = (usernameInput && usernameInput.value || '').trim();
      var password = passwordInput && passwordInput.value || '';
      if (!username || !password) return;
      setStatus('signing in', 'idle');
      apiJson('/api/auth/login', {
        method: 'POST',
        body: JSON.stringify({ username: username, password: password })
      }).then(function() {
        if (passwordInput) passwordInput.value = '';
        hideAuth();
        return loadConfig();
      }).catch(function() {
        showAuth('Login failed. Check the configured web username and password.');
        setStatus('login failed', 'error');
      });
    });
  }

  editor.addEventListener('input', updateStats);
  searchInput.addEventListener('input', renderSections);
  reloadBtn.addEventListener('click', loadConfig);
  validateBtn.addEventListener('click', validateConfig);
  saveBtn.addEventListener('click', saveConfig);
  var templateConfirmTimer = null;
  var templateConfirmLabel = insertTemplateBtn.textContent;
  insertTemplateBtn.addEventListener('click', function() {
    if (!template) return;
    if (insertTemplateBtn.dataset.confirm !== '1') {
      insertTemplateBtn.dataset.confirm = '1';
      insertTemplateBtn.textContent = 'Click again to overwrite';
      templateConfirmTimer = setTimeout(function() {
        insertTemplateBtn.dataset.confirm = '';
        insertTemplateBtn.textContent = templateConfirmLabel;
      }, 3000);
      return;
    }
    clearTimeout(templateConfirmTimer);
    insertTemplateBtn.dataset.confirm = '';
    insertTemplateBtn.textContent = templateConfirmLabel;
    editor.value = template;
    updateStats();
    setStatus('template loaded into editor', 'idle');
  });

  setStatus('checking access', 'idle');
  checkAccess().then(function(ok) {
    if (ok) loadConfig();
  });
})();
