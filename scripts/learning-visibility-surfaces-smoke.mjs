#!/usr/bin/env node

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staticRoot = join(repoRoot, 'crates/captain-api/static');
const controlVendorAliases = new Map([
  ['/assets/app/vendor/preact.module.js', 'vendor/preact/preact.module.js'],
  ['/assets/app/vendor/hooks.module.js', 'vendor/preact/hooks.module.js'],
  ['/assets/app/vendor/htm.module.js', 'vendor/preact/htm.module.js'],
  ['/assets/app/vendor/marked.esm.js', 'vendor/marked/marked.esm.js'],
  ['/assets/app/vendor/purify.es.mjs', 'vendor/dompurify/purify.es.mjs'],
]);

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
    "preact":"/assets/app/vendor/preact.module.js",
    "preact/hooks":"/assets/app/vendor/hooks.module.js",
    "htm":"/assets/app/vendor/htm.module.js"
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
  proposal_id: 'proposal-1', state: 'drafting', kind: 'skill', projection_status: 'verified',
  revision_sha256: 'a'.repeat(64), timeline: [{ kind: 'proposal' }],
  card: {
    name: 'veille-technologique', purpose: 'Conserver une veille réutilisable et vérifiée.',
    state: 'drafting', trigger: 'Demande de veille ciblée', evidence: { occurrences: 4, distinct_sessions: 2 },
    validation: [{ code: 'schema', passed: true }], validated_by: { provider: 'codex', model: 'gpt-5.6-sol' },
    required_authority: [], available_actions: ['activate', 'test', 'later', 'ignore'],
    lookup_token: 'abcdefghijklmnopqrst', decision_version: 3,
  },
};

const fixtures = {
  '/api/learning/review': { pending: [] },
  '/api/learning/committed': { committed: [] },
  '/api/learning/metrics': {
    review_queue_pending: 0, learning_mode: 'approval', learning_enabled: true,
    memory_writes: { total: 50, synced: 50, pending: 0, error: 0, recovery: 'in_sync' },
  },
  '/api/learning/workflows': { schema_version: 1, returned: 1, workflows: [workflow] },
  '/api/learning/status': {
    schema_version: 2, enabled: true, mode: 'approval', state: 'degraded',
    recovery: 'operator_attention', generated_at_unix_ms: 100000,
    expected_model: { provider: 'codex', model: 'gpt-5.6-sol' },
    worker: {
      phase: 'running', bound_model: { provider: 'codex', model: 'gpt-5.6-sol' },
      started_at_unix_ms: 1000, heartbeat_at_unix_ms: 98000, heartbeat_age_ms: 2000,
      last_scan_at_unix_ms: 95000, last_progress_at_unix_ms: 90000, last_error_scope: null,
    },
    jobs: {
      pending: 0, running: 0, retry_wait: 0, uncertain: 0, dead: 1,
      oldest_actionable_at_unix_ms: null, next_retry_at_unix_ms: null,
      last_activity_at_unix_ms: 99000, last_error_code: 'model_timeout',
    },
    notifications: {
      pending: 1, delivering: 0, retry_wait: 1, dead: 0,
      oldest_actionable_at_unix_ms: 85000, next_retry_at_unix_ms: 120000,
      last_activity_at_unix_ms: 99000,
    },
    workflows: { total: 1, processing: 0, awaiting_decision: 0, active: 0, attention: 1, last_activity_at_unix_ms: 99000 },
    attention: [{
      proposal_id: 'proposal-1', stage: 'draft', state: 'dead', error_code: 'model_timeout',
      attempt_count: 3, max_attempts: 3, retry_available: true, updated_at_unix_ms: 99000,
    }],
  },
};

const retryRequests = [];

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

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
  if (request.method === 'POST' && pathname === '/api/learning/workflows/proposal-1/retry') {
    retryRequests.push(await readJson(request));
    return send(response, 200, 'application/json', JSON.stringify({
      schema_version: 1,
      proposal_id: 'proposal-1',
      job_id: 'job-1',
      state: 'pending',
      replayed: false,
    }));
  }
  if (fixtures[pathname]) return send(response, 200, 'application/json', JSON.stringify(fixtures[pathname]));
  if (controlVendorAliases.has(pathname)) {
    const target = resolve(staticRoot, controlVendorAliases.get(pathname));
    return send(response, 200, contentType(target), await readFile(target));
  }
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
    await page.locator('[data-learning-state="degraded"]').waitFor();
    const snapshot = await page.evaluate(() => {
      const strip = document.querySelector('.learning-runtime-strip');
      const alert = document.querySelector('.learning-runtime-alert');
      const rect = strip?.getBoundingClientRect();
      return {
        text: strip?.textContent || '',
        alertText: alert?.textContent || '',
        pageText: document.body.textContent || '',
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
    assert.match(snapshot.text, /Learning à vérifier/);
    assert.match(snapshot.text, /codex:gpt-5\.6-sol/);
    assert.match(snapshot.text, /0 en attente · 0 actif/);
    assert.match(snapshot.text, /0 retry · 0 incertain · 1 bloqué/);
    assert.match(snapshot.text, /relance automatique arrêtée après épuisement des essais/);
    assert.doesNotMatch(snapshot.text, /0\/0\/1 jobs/);
    assert.match(snapshot.alertText, /Génération arrêtée/);
    assert.match(snapshot.alertText, /génération · délai du modèle dépassé · 3\/3 essais/);
    assert.match(snapshot.alertText, /La mémoire durable reste active et n’est pas affectée/);
    assert.match(snapshot.pageText, /1 à examiner/);
    assert.match(snapshot.pageText, /0 en cours/);
    assert.match(snapshot.pageText, /mémoire active/);
    assert.match(snapshot.pageText, /veille-technologique/);
    assert.doesNotMatch(snapshot.text, /%/);
    if (surface.name === 'desktop') {
      await Promise.all([
        page.waitForResponse((response) => response.url().endsWith('/api/learning/workflows/proposal-1/retry') && response.status() === 200),
        page.getByRole('button', { name: 'Relancer ce workflow' }).click(),
      ]);
      assert.deepEqual(retryRequests, [{ expected_error_code: 'model_timeout', surface: 'web' }]);
    }
    await page.screenshot({
      path: `/private/tmp/captain-learning-status-${surface.name}.png`,
      fullPage: true,
    });
    await page.close();
  }
  fixtures['/api/learning/metrics'] = {
    review_queue_pending: 0, learning_mode: 'approval', learning_enabled: true,
    memory_writes: { total: 50, synced: 49, pending: 1, error: 0, recovery: 'automatic_retry_active' },
  };
  const recoveryPage = await browser.newPage({ viewport: { width: 1280, height: 720 } });
  await recoveryPage.goto(`http://127.0.0.1:${port}`, { waitUntil: 'networkidle' });
  await recoveryPage.locator('[data-learning-state="degraded"]').waitFor();
  const recoveryText = await recoveryPage.locator('body').innerText();
  assert.match(recoveryText, /mémoire en reprise/);
  assert.match(recoveryText, /distinct de l’état de la mémoire durable/);
  assert.doesNotMatch(recoveryText, /La mémoire durable reste active et n’est pas affectée/);
  await recoveryPage.close();
  console.log('Learning visibility Control/Desktop surfaces smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
