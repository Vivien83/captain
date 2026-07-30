#!/usr/bin/env node

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staticRoot = join(repoRoot, 'crates/captain-api/static');
const sessionId = '11111111-1111-4111-8111-111111111111';
const agentId = '22222222-2222-4222-8222-222222222222';

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

const [themeCss, xtermCss, terminalCss, body, xtermJs, unicodeJs, fitJs, terminalJs, logoPng] =
  await Promise.all([
    readFile(join(staticRoot, 'css/theme.css'), 'utf8'),
    readFile(join(staticRoot, 'vendor/xterm/xterm.css'), 'utf8'),
    readFile(join(staticRoot, 'css/terminal.css'), 'utf8'),
    readFile(join(staticRoot, 'terminal_body.html'), 'utf8'),
    readFile(join(staticRoot, 'vendor/xterm/xterm.js'), 'utf8'),
    readFile(join(staticRoot, 'vendor/xterm/addon-unicode11.js'), 'utf8'),
    readFile(join(staticRoot, 'vendor/xterm/addon-fit.js'), 'utf8'),
    readFile(join(staticRoot, 'js/pages/terminal.js'), 'utf8'),
    readFile(join(repoRoot, 'assets/logo.png')),
  ]);

const mockRuntime = `<script>
class FakeWebSocket {
  static OPEN = 1;
  static CLOSED = 3;
  constructor() {
    this.readyState = 0;
    queueMicrotask(() => {
      this.readyState = FakeWebSocket.OPEN;
      if (this.onopen) this.onopen();
    });
  }
  send() {}
  close() {
    this.readyState = FakeWebSocket.CLOSED;
    if (this.onclose) this.onclose();
  }
}
window.WebSocket = FakeWebSocket;
window.__captainFetches = [];

const canonicalSessionId = '${sessionId}';
const captainAgentId = '${agentId}';
const now = Date.now();
const compactionEvents = [
  {
    id: 901,
    session_id: canonicalSessionId,
    ts: now - 1000,
    event_type: 'compaction_progress',
    payload: {
      schema_version: 1,
      operation_id: 'compact-terminal',
      runtime_instance_id: 'runtime-terminal',
      agent_id: captainAgentId,
      session_id: canonicalSessionId,
      phase: 'summarizing',
      state: 'running',
      detail: 'Appel modèle opaque sans métrique interne',
      message_count: 180,
      estimated_tokens: 42000,
      context_window_tokens: 200000,
      completed_units: null,
      total_units: null,
      unit: null,
      started_at_ms: now - 2000,
      updated_at_ms: now - 1000,
    },
  },
  {
    id: 902,
    session_id: canonicalSessionId,
    ts: now,
    event_type: 'compaction_progress',
    payload: {
      schema_version: 1,
      operation_id: 'compact-terminal',
      runtime_instance_id: 'runtime-terminal',
      agent_id: captainAgentId,
      session_id: canonicalSessionId,
      phase: 'chunking',
      state: 'running',
      detail: 'Deux lots vérifiés sur quatre',
      message_count: 180,
      estimated_tokens: 42000,
      context_window_tokens: 200000,
      completed_units: 2,
      total_units: 4,
      unit: 'chunks',
      started_at_ms: now - 2000,
      updated_at_ms: now,
    },
  },
];

window.fetch = async (input) => {
  const url = new URL(typeof input === 'string' ? input : input.url, location.origin);
  window.__captainFetches.push(url.pathname + url.search);
  let body = {};
  if (url.pathname === '/api/auth/check') {
    body = { mode: 'session', authenticated: true };
  } else if (url.pathname === '/api/terminal/sessions') {
    body = { sessions: [] };
  } else if (url.pathname === '/api/sessions') {
    body = { sessions: [] };
  } else if (url.pathname === '/api/agents') {
    body = { agents: [{ id: captainAgentId, name: 'captain' }] };
  } else if (url.pathname === '/api/agents/' + captainAgentId + '/sessions') {
    body = { sessions: [{ session_id: canonicalSessionId, active: true }] };
  } else if (url.pathname === '/api/sessions/' + canonicalSessionId + '/events') {
    body = { events: compactionEvents };
  } else if (url.pathname === '/api/usage/summary') {
    body = { total_input_tokens: 0, total_output_tokens: 0, total_cost_usd: 0, call_count: 0 };
  }
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'content-type': 'application/json' },
  });
};
</script>`;

const pageHtml = `<!doctype html>
<html lang="fr"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Captain terminal compaction smoke</title>
<style>${themeCss}\n${xtermCss}\n${terminalCss}</style>
</head><body class="terminal-body" data-theme="dark">
${body}
<script>${xtermJs}</script>
<script>${unicodeJs}</script>
<script>${fitJs}</script>
${mockRuntime}
<script>${terminalJs}</script>
</body></html>`;

const server = createServer((request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/assets/logo.png') {
    response.writeHead(200, { 'content-type': 'image/png', 'cache-control': 'no-store' });
    response.end(logoPng);
    return;
  }
  if (pathname !== '/') {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, {
    'content-type': 'text/html; charset=utf-8',
    'cache-control': 'no-store',
  });
  response.end(pageHtml);
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
    await page.goto(`http://127.0.0.1:${port}`, { waitUntil: 'load' });
    try {
      await page.waitForFunction(() =>
        [...document.querySelectorAll('.terminal-activity-card')]
          .some((element) => element.textContent.includes('2/4 lots')),
      );
    } catch (error) {
      const diagnostic = await page.evaluate(() => ({
        fetches: window.__captainFetches || [],
        status: document.querySelector('#status-text')?.textContent || '',
        activity: document.querySelector('#activity-list')?.textContent || '',
      }));
      throw new Error(`${surface.name}: compaction progress did not render: ${JSON.stringify({ pageErrors, diagnostic })}`, {
        cause: error,
      });
    }
    if (surface.name === 'mobile') {
      await page.locator('#activity-toggle').click();
    }
    await page.waitForTimeout(650);
    const snapshot = await page.evaluate(() => {
      const cards = [...document.querySelectorAll('.terminal-activity-card')];
      const card = cards.find((element) => element.textContent.includes('2/4 lots'));
      const bar = card?.querySelector('[role="progressbar"]');
      const rect = card?.getBoundingClientRect();
      return {
        matchingCards: cards.filter((element) => element.textContent.includes('Context compaction')).length,
        text: card?.textContent || '',
        ariaNow: bar?.getAttribute('aria-valuenow') || null,
        visible: Boolean(rect && rect.width > 0 && rect.height > 0 && rect.top < window.innerHeight),
        clientWidth: document.documentElement.clientWidth,
        scrollWidth: document.documentElement.scrollWidth,
        overflowing: [...document.querySelectorAll('body *')]
          .filter((element) => {
            const style = window.getComputedStyle(element);
            if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) return false;
            const bounds = element.getBoundingClientRect();
            return bounds.width > 0 && bounds.height > 0
              && (bounds.left < -0.5 || bounds.right > document.documentElement.clientWidth + 0.5);
          })
          .map((element) => `${element.tagName}.${element.className}`)
          .slice(0, 20),
      };
    });
    assert.equal(pageErrors.length, 0, `${surface.name}: ${pageErrors.join('; ')}`);
    assert.equal(snapshot.matchingCards, 1, `${surface.name}: progress operation was duplicated`);
    assert.match(snapshot.text, /2\/4 lots · 50%/);
    assert.equal(snapshot.ariaNow, '50');
    assert.equal(snapshot.visible, true, `${surface.name}: progress card is not visible`);
    assert.equal(snapshot.scrollWidth, snapshot.clientWidth, `${surface.name}: horizontal overflow`);
    assert.deepEqual(snapshot.overflowing, [], `${surface.name}: elements leave viewport`);
    await page.screenshot({
      path: `/private/tmp/captain-terminal-compaction-${surface.name}.png`,
      fullPage: false,
    });
    await page.close();
  }
  console.log('Web terminal compaction progress smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
