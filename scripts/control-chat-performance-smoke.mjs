#!/usr/bin/env node

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staticRoot = join(repoRoot, 'crates/captain-api/static');

async function importPlaywright() {
  const candidates = [process.env.CAPTAIN_PLAYWRIGHT_MODULE, 'playwright'].filter(Boolean);
  try {
    const npmRoot = execFileSync('npm', ['root', '-g'], { encoding: 'utf8' }).trim();
    candidates.push(join(npmRoot, 'playwright/index.mjs'), join(npmRoot, 'playwright/index.js'));
  } catch {
    // A repository-local Playwright install can satisfy the package import.
  }
  const errors = [];
  for (const candidate of candidates) {
    try {
      return await import(candidate.startsWith('/') ? pathToFileURL(candidate).href : candidate);
    } catch (error) {
      errors.push(`${candidate}: ${error.message}`);
    }
  }
  throw new Error(`Playwright is required. Tried:\n${errors.join('\n')}`);
}

const appHtml = `<!doctype html>
<html lang="fr">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Captain Control chat performance smoke</title>
  <link rel="stylesheet" href="/static/css/theme.css">
  <link rel="stylesheet" href="/static/css/app.css">
  <script type="importmap">
  {"imports":{
    "preact":"/static/vendor/preact/preact.module.js",
    "preact/hooks":"/static/vendor/preact/hooks.module.js",
    "htm":"/static/vendor/preact/htm.module.js",
    "marked":"/static/vendor/marked/marked.esm.js",
    "dompurify":"/static/vendor/dompurify/purify.es.mjs"
  }}
  </script>
</head>
<body data-theme="dark"><div id="app"></div>
<script type="module">
  import { h, render } from 'preact';
  import htm from 'htm';
  import { setState } from '/static/js/app/store.js';
  import { Chat } from '/static/js/app/views/Chat.js';

  const html = htm.bind(h);
  const events = [];
  for (let index = 0; index < 120; index += 1) {
    events.push({ event_type: 'user_message', payload: {
      content: 'Question ' + index + ' avec une contrainte opérationnelle précise.'
    }});
    events.push({ event_type: 'assistant_message', payload: {
      content: index % 12 === 0
        ? '## Résultat ' + index + '\\n\\n| Signal | État |\\n|---|---|\\n| Exactitude | OK |\\n\\npreuve-' + index
        : 'Réponse vérifiée ' + index + ' avec contexte, preuve et prochaine action.'
    }});
  }

  class FakeWebSocket {
    static OPEN = 1;
    static CLOSED = 3;
    constructor() {
      this.readyState = 0;
      FakeWebSocket.instances.push(this);
      queueMicrotask(() => {
        this.readyState = FakeWebSocket.OPEN;
        if (this.onopen) this.onopen();
      });
    }
    emit(message) {
      if (this.onmessage) this.onmessage({ data: JSON.stringify(message) });
    }
    send(payload) { this.lastSent = payload; }
    close() {
      this.readyState = FakeWebSocket.CLOSED;
      if (this.onclose) this.onclose();
    }
  }
  FakeWebSocket.instances = [];
  window.WebSocket = FakeWebSocket;
  window.__captainFakeSockets = FakeWebSocket.instances;

  window.fetch = async (input) => {
    const url = new URL(typeof input === 'string' ? input : input.url, location.origin);
    let body = {};
    if (url.pathname === '/api/budget') {
      body = { provider_subscriptions: { state: 'ok', reported_by_provider: true, items: [] }};
    } else if (url.pathname.endsWith('/reasoning')) {
      body = { configured_effort: null, effective_effort: 'high', available_efforts: ['low', 'high'] };
    } else if (url.pathname.endsWith('/sessions')) {
      body = { sessions: [{ active: true, session_id: 'perf-session' }] };
    } else if (url.pathname.includes('/api/sessions/perf-session/events')) {
      body = { events };
    }
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  };

  setState({
    currentAgentId: 'perf-agent',
    agents: [{ id: 'perf-agent', model_provider: 'codex', model_name: 'gpt-5.6-sol' }],
  });
  window.__captainPerfStartedAt = performance.now();
  render(html\`<div class="shell"><div class="main"><\${Chat} /></div></div>\`, document.getElementById('app'));
</script></body></html>`;

function send(response, status, type, body) {
  response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  response.end(body);
}

function contentType(path) {
  switch (extname(path)) {
    case '.css': return 'text/css; charset=utf-8';
    case '.js':
    case '.mjs': return 'text/javascript; charset=utf-8';
    default: return 'application/octet-stream';
  }
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (!pathname.startsWith('/static/')) return send(response, 404, 'text/plain', 'not found');
  const relative = normalize(decodeURIComponent(pathname.slice('/static/'.length)));
  const target = resolve(staticRoot, relative);
  if (relative.startsWith('..') || !target.startsWith(`${staticRoot}/`)) {
    return send(response, 404, 'text/plain', 'not found');
  }
  try {
    return send(response, 200, contentType(target), await readFile(target));
  } catch {
    return send(response, 404, 'text/plain', 'not found');
  }
});

await new Promise((resolveListen, rejectListen) => {
  server.once('error', rejectListen);
  server.listen(0, '127.0.0.1', resolveListen);
});

let browser;
try {
  const { chromium } = await importPlaywright();
  browser = await chromium.launch({ headless: true });
  const port = server.address().port;
  for (const surface of [
    { name: 'desktop', viewport: { width: 1440, height: 900 } },
    { name: 'mobile', viewport: { width: 390, height: 844 } },
  ]) {
    const page = await browser.newPage({ viewport: surface.viewport });
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    await page.goto(`http://127.0.0.1:${port}`, { waitUntil: 'networkidle' });
    await page.waitForFunction(() => document.querySelectorAll('.msg').length === 240);

    const hydration = await page.evaluate(() => ({
      elapsedMs: performance.now() - window.__captainPerfStartedAt,
      settledAnimation: getComputedStyle(document.querySelector('.msg')).animationName,
      contentVisibility: getComputedStyle(document.querySelector('.msg')).contentVisibility,
    }));
    assert.ok(hydration.elapsedMs < 2500, `${surface.name}: transcript hydration ${hydration.elapsedMs}ms`);
    assert.equal(hydration.settledAnimation, 'none', `${surface.name}: replay animates every row`);
    assert.equal(hydration.contentVisibility, 'auto', `${surface.name}: offscreen rows are not contained`);

    await page.evaluate(() => {
      const scroll = document.querySelector('.chat-scroll');
      scroll.scrollTop = Math.max(0, scroll.scrollHeight - scroll.clientHeight - 700);
      scroll.dispatchEvent(new Event('scroll'));
      const socket = window.__captainFakeSockets.at(-1);
      socket.emit({ type: 'catch_up', is_streaming: true, user_message: 'Scrollback guard', accumulated_text: '' });
      for (let index = 0; index < 500; index += 1) {
        socket.emit({ type: 'text_delta', content: String(index % 10) });
      }
    });
    await page.waitForTimeout(100);
    const scrollbackDistance = await page.evaluate(() => {
      const scroll = document.querySelector('.chat-scroll');
      return scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight;
    });
    assert.ok(
      scrollbackDistance >= 300,
      `${surface.name}: streaming overrode operator scrollback (${scrollbackDistance}px)`,
    );

    await page.evaluate(() => {
      const scroll = document.querySelector('.chat-scroll');
      scroll.scrollTop = scroll.scrollHeight;
      scroll.dispatchEvent(new Event('scroll'));
      const target = document.querySelector('.chat-inner');
      window.__captainMutationCallbacks = 0;
      window.__captainMutationObserver = new MutationObserver(() => {
        window.__captainMutationCallbacks += 1;
      });
      window.__captainMutationObserver.observe(target, {
        subtree: true, childList: true, characterData: true,
      });
      const socket = window.__captainFakeSockets.at(-1);
      socket.emit({ type: 'catch_up', is_streaming: true, user_message: 'Burst exact', accumulated_text: '' });
      for (let index = 0; index < 1000; index += 1) {
        socket.emit({ type: 'text_delta', content: String(index % 10) });
      }
    });
    await page.waitForTimeout(200);

    await page.evaluate(() => {
      const socket = window.__captainFakeSockets.at(-1);
      const base = {
        schema_version: 1,
        operation_id: 'compact-visual',
        runtime_instance_id: 'runtime-visual',
        agent_id: 'perf-agent',
        session_id: 'perf-session',
        state: 'running',
        detail: 'Synthèse exacte du contexte actif',
        message_count: 240,
        estimated_tokens: 48000,
        context_window_tokens: 200000,
        started_at_ms: 1,
        updated_at_ms: 2,
      };
      socket.emit({
        type: 'compaction_progress',
        progress: { ...base, phase: 'summarizing', completed_units: null, total_units: null, unit: null },
      });
    });
    await page.locator('.compaction-progress').waitFor();
    const opaqueProgress = await page.locator('.compaction-progress').evaluate((element) => ({
      text: element.textContent,
      now: element.querySelector('[role="progressbar"]').getAttribute('aria-valuenow'),
    }));
    assert.match(opaqueProgress.text, /progression indéterminée/);
    assert.doesNotMatch(opaqueProgress.text, /%/);
    assert.equal(opaqueProgress.now, null, `${surface.name}: opaque work exposes a fake percentage`);

    await page.evaluate(() => {
      const socket = window.__captainFakeSockets.at(-1);
      const progress = {
        schema_version: 1,
        operation_id: 'compact-visual',
        runtime_instance_id: 'runtime-visual',
        agent_id: 'perf-agent',
        session_id: 'perf-session',
        phase: 'chunking',
        state: 'running',
        detail: 'Deux lots vérifiés sur quatre',
        message_count: 240,
        estimated_tokens: 48000,
        context_window_tokens: 200000,
        completed_units: 2,
        total_units: 4,
        unit: 'chunks',
        started_at_ms: 1,
        updated_at_ms: 3,
      };
      socket.emit({ type: 'compaction_progress', progress });
      socket.emit({
        type: 'compaction_progress',
        progress: { ...progress, session_id: 'another-session', completed_units: 4, updated_at_ms: 4 },
      });
    });
    await page.waitForFunction(() => document.querySelector('.compaction-progress')?.textContent.includes('2/4 lots'));

    const snapshot = await page.evaluate(() => {
      window.__captainMutationObserver.disconnect();
      const messages = document.querySelectorAll('.msg.assistant .md');
      const scroll = document.querySelector('.chat-scroll');
      const composer = document.querySelector('.composer-wrap').getBoundingClientRect();
      const quota = document.querySelector('.provider-quota-bar').getBoundingClientRect();
      const compaction = document.querySelector('.compaction-progress').getBoundingClientRect();
      const compactionText = document.querySelector('.compaction-progress').textContent;
      const compactionNow = document.querySelector('.compaction-progress [role="progressbar"]').getAttribute('aria-valuenow');
      return {
        finalText: messages[messages.length - 1].textContent,
        mutationCallbacks: window.__captainMutationCallbacks,
        clientWidth: document.documentElement.clientWidth,
        scrollWidth: document.documentElement.scrollWidth,
        pinnedDistance: scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight,
        composerBottom: composer.bottom,
        quotaBottom: quota.bottom,
        compactionBottom: compaction.bottom,
        compactionText,
        compactionNow,
        overflowing: [...document.querySelectorAll('body *')]
          .filter((element) => {
            const rect = element.getBoundingClientRect();
            return rect.left < -0.5 || rect.right > document.documentElement.clientWidth + 0.5;
          })
          .map((element) => `${element.tagName}.${element.className}`)
          .slice(0, 20),
      };
    });
    assert.equal(snapshot.finalText.trimEnd(), '0123456789'.repeat(100), `${surface.name}: delta loss/reordering`);
    assert.ok(snapshot.mutationCallbacks <= 4, `${surface.name}: ${snapshot.mutationCallbacks} DOM commit batches`);
    assert.equal(snapshot.scrollWidth, snapshot.clientWidth, `${surface.name}: horizontal overflow`);
    assert.deepEqual(snapshot.overflowing, [], `${surface.name}: elements leave viewport`);
    assert.ok(
      snapshot.pinnedDistance < 4,
      `${surface.name}: streaming lost bottom pin (${snapshot.pinnedDistance}px)`,
    );
    assert.ok(snapshot.composerBottom <= surface.viewport.height, `${surface.name}: composer hidden`);
    assert.ok(snapshot.quotaBottom <= surface.viewport.height, `${surface.name}: quota hidden`);
    assert.ok(snapshot.compactionBottom <= surface.viewport.height, `${surface.name}: compaction status hidden`);
    assert.match(snapshot.compactionText, /2\/4 lots · 50%/);
    assert.equal(snapshot.compactionNow, '50');

    await page.evaluate(() => {
      const socket = window.__captainFakeSockets.at(-1);
      socket.emit({
        type: 'catch_up',
        is_streaming: true,
        user_message: 'Préférence de première utilisation',
        accumulated_text: '',
      });
      socket.emit({
        type: 'suggested_replies',
        options: ['Court et direct', 'Détaillé avec les points importants'],
      });
      socket.emit({ type: 'text_delta', content: 'Quel style de réponse préfères-tu ?' });
      socket.emit({ type: 'response', content: '' });
    });
    await page.locator('.suggested-replies button').first().waitFor();
    await page.waitForTimeout(320);
    const suggestionLayout = await page.locator('.suggested-replies').evaluate((element) => ({
      clientWidth: document.documentElement.clientWidth,
      messageOpacity: getComputedStyle(element.closest('.msg')).opacity,
      scrollDistance: (() => {
        const scroll = document.querySelector('.chat-scroll');
        return scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight;
      })(),
      scrollBottom: document.querySelector('.chat-scroll').getBoundingClientRect().bottom,
      buttons: [...element.querySelectorAll('button')].map((button) => {
        const rect = button.getBoundingClientRect();
        return { left: rect.left, right: rect.right, bottom: rect.bottom, width: rect.width };
      }),
    }));
    assert.equal(suggestionLayout.messageOpacity, '1', `${surface.name}: suggestions remain faded`);
    assert.ok(
      suggestionLayout.scrollDistance < 4,
      `${surface.name}: suggestions lost bottom pin (${suggestionLayout.scrollDistance}px)`,
    );
    assert.ok(
      suggestionLayout.buttons.every((button) =>
        button.left >= 0 &&
        button.right <= suggestionLayout.clientWidth &&
        button.bottom <= suggestionLayout.scrollBottom &&
        button.width > 0),
      `${surface.name}: suggested reply leaves viewport`,
    );
    await page.screenshot({
      path: `/private/tmp/captain-control-chat-suggestions-${surface.name}.png`,
      fullPage: false,
    });
    await page.locator('.suggested-replies button').nth(1).click();
    await page.waitForFunction(() => document.querySelectorAll('.suggested-replies button').length === 0);
    const suggestedReplySend = await page.evaluate(() => ({
      payload: JSON.parse(window.__captainFakeSockets.at(-1).lastSent),
      lastUserText: [...document.querySelectorAll('.msg.user .md')].at(-1)?.textContent.trim(),
    }));
    assert.deepEqual(
      suggestedReplySend.payload,
      { type: 'message', content: 'Détaillé avec les points importants' },
      `${surface.name}: suggested reply did not use normal message ingress`,
    );
    assert.equal(
      suggestedReplySend.lastUserText,
      'Détaillé avec les points importants',
      `${surface.name}: selected suggestion missing from transcript`,
    );

    await page.evaluate(() => {
      const socket = window.__captainFakeSockets.at(-1);
      socket.emit({
        type: 'broadcast',
        event: {
          chat_event: 'UserMessage',
          content: 'Préférence reçue depuis Telegram',
          channel: 'telegram',
        },
      });
      socket.emit({
        type: 'broadcast',
        event: {
          chat_event: 'SuggestedReplies',
          options: ['Toujours demander', 'Jamais de données sensibles'],
        },
      });
      socket.emit({
        type: 'broadcast',
        event: {
          chat_event: 'TextDelta',
          delta: 'Quelle règle de confidentialité dois-je respecter ?',
        },
      });
      socket.emit({
        type: 'broadcast',
        event: {
          chat_event: 'Response',
          content: 'Quelle règle de confidentialité dois-je respecter ?',
        },
      });
    });
    await page.locator('.suggested-replies button').first().waitFor();
    const crossSurfaceQuestion = await page.evaluate(() =>
      [...document.querySelectorAll('.msg.assistant .md')].at(-1)?.textContent.trim());
    assert.equal(
      crossSurfaceQuestion,
      'Quelle règle de confidentialité dois-je respecter ?',
      `${surface.name}: tagged cross-surface response was not mirrored`,
    );
    await page.locator('.suggested-replies button').first().click();
    await page.waitForFunction(() => document.querySelectorAll('.suggested-replies button').length === 0);
    const crossSurfaceSend = await page.evaluate(() =>
      JSON.parse(window.__captainFakeSockets.at(-1).lastSent));
    assert.deepEqual(
      crossSurfaceSend,
      { type: 'message', content: 'Toujours demander' },
      `${surface.name}: cross-surface suggestion did not use normal ingress`,
    );

    assert.equal(pageErrors.length, 0, `${surface.name}: ${pageErrors.join('; ')}`);
    await page.screenshot({
      path: `/private/tmp/captain-control-chat-performance-${surface.name}.png`,
      fullPage: false,
    });
    await page.close();
  }
  console.log('Control/Desktop chat performance smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
