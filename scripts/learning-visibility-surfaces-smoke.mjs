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
  <title>Captain Learning visibility smoke</title>
  <link rel="stylesheet" href="/static/css/theme.css">
  <link rel="stylesheet" href="/static/css/app.css">
  <script type="importmap">
  {"imports":{
    "preact":"/static/vendor/preact/preact.module.js",
    "preact/hooks":"/static/vendor/preact/hooks.module.js",
    "htm":"/static/vendor/preact/htm.module.js"
  }}
  </script>
</head>
<body data-theme="dark"><div id="app"></div>
<script type="module">
  import { h, render } from 'preact';
  import htm from 'htm';
  import { Learning } from '/static/js/app/views/Learning.js';
  const html = htm.bind(h);
  render(html\`<\${Learning} />\`, document.getElementById('app'));
</script></body></html>`;

const workflow = {
  proposal_id: 'proposal-1', state: 'proposed', kind: 'skill', projection_status: 'verified',
  revision_sha256: 'a'.repeat(64), timeline: [{ kind: 'proposal' }],
  card: {
    name: 'veille-technologique', purpose: 'Conserver une veille réutilisable et vérifiée.',
    state: 'proposed', trigger: 'Demande de veille ciblée', evidence: { occurrences: 4, distinct_sessions: 2 },
    validation: [{ code: 'schema', passed: true }], validated_by: { provider: 'codex', model: 'gpt-5.6-sol' },
    required_authority: [], available_actions: ['activate', 'test', 'later', 'ignore'],
    lookup_token: 'abcdefghijklmnopqrst', decision_version: 3,
  },
};

const fixtures = {
  '/api/learning/review': { pending: [] },
  '/api/learning/committed': { committed: [] },
  '/api/learning/metrics': { review_queue_pending: 0, learning_mode: 'approval', learning_enabled: true },
  '/api/learning/workflows': { schema_version: 1, returned: 1, workflows: [workflow] },
  '/api/learning/status': {
    schema_version: 1, enabled: true, mode: 'approval', state: 'recovering',
    recovery: 'automatic_retry_active', generated_at_unix_ms: 100000,
    expected_model: { provider: 'codex', model: 'gpt-5.6-sol' },
    worker: {
      phase: 'running', bound_model: { provider: 'codex', model: 'gpt-5.6-sol' },
      started_at_unix_ms: 1000, heartbeat_at_unix_ms: 98000, heartbeat_age_ms: 2000,
      last_scan_at_unix_ms: 95000, last_progress_at_unix_ms: 90000, last_error_scope: null,
    },
    jobs: {
      pending: 1, running: 2, retry_wait: 3, uncertain: 0, dead: 0,
      oldest_actionable_at_unix_ms: 80000, next_retry_at_unix_ms: 130000,
      last_activity_at_unix_ms: 99000, last_error_code: 'provider_busy',
    },
    notifications: {
      pending: 1, delivering: 0, retry_wait: 1, dead: 0,
      oldest_actionable_at_unix_ms: 85000, next_retry_at_unix_ms: 120000,
      last_activity_at_unix_ms: 99000,
    },
    workflows: { total: 4, processing: 1, awaiting_decision: 1, active: 2, attention: 0, last_activity_at_unix_ms: 99000 },
  },
};

function send(response, status, type, body) {
  response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  response.end(body);
}

function contentType(path) {
  switch (extname(path)) {
    case '.css': return 'text/css; charset=utf-8';
    case '.js':
    case '.mjs': return 'text/javascript; charset=utf-8';
    case '.png': return 'image/png';
    case '.svg': return 'image/svg+xml';
    default: return 'application/octet-stream';
  }
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (fixtures[pathname]) return send(response, 200, 'application/json', JSON.stringify(fixtures[pathname]));
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
    await page.locator('[data-learning-state="recovering"]').waitFor();
    const snapshot = await page.evaluate(() => {
      const strip = document.querySelector('.learning-runtime-strip');
      const rect = strip?.getBoundingClientRect();
      return {
        text: strip?.textContent || '',
        clientWidth: document.documentElement.clientWidth,
        scrollWidth: document.documentElement.scrollWidth,
        stripVisible: Boolean(rect && rect.width > 0 && rect.height > 0 && rect.top < window.innerHeight),
        overflowing: [...document.querySelectorAll('body *')]
          .filter((element) => {
            const bounds = element.getBoundingClientRect();
            return bounds.left < -0.5 || bounds.right > document.documentElement.clientWidth + 0.5;
          })
          .map((element) => `${element.tagName}.${element.className}`)
          .slice(0, 20),
      };
    });
    assert.equal(pageErrors.length, 0, `${surface.name}: ${pageErrors.join('; ')}`);
    assert.equal(snapshot.scrollWidth, snapshot.clientWidth, `${surface.name}: horizontal overflow`);
    assert.deepEqual(snapshot.overflowing, [], `${surface.name}: elements leave viewport`);
    assert.equal(snapshot.stripVisible, true, `${surface.name}: status is not visible in first viewport`);
    assert.match(snapshot.text, /Reprise automatique/);
    assert.match(snapshot.text, /codex:gpt-5\.6-sol/);
    assert.match(snapshot.text, /1\/2\/3 jobs/);
    assert.match(snapshot.text, /prochain retry dans 30s/);
    assert.doesNotMatch(snapshot.text, /%/);
    await page.screenshot({
      path: `/private/tmp/captain-learning-status-${surface.name}.png`,
      fullPage: true,
    });
    await page.close();
  }
  console.log('Learning visibility Control/Desktop surfaces smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
