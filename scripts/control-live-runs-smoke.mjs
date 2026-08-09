#!/usr/bin/env node

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { dirname, extname, join, normalize, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staticRoot = join(repoRoot, 'crates/captain-api/static');
const middlewareSource = await readFile(
  join(repoRoot, 'crates/captain-api/src/middleware.rs'),
  'utf8',
);
const cspMatch = middlewareSource.match(/CONTENT_SECURITY_POLICY: &str = "([^"]+)"/);
assert.ok(cspMatch, 'the production CSP constant must remain inspectable');
const appCsp = cspMatch[1];
const forbiddenSecret = 'sk-live-runs-secret-must-never-render';
const now = Date.now();

const baseRuns = [
  runFixture('toolrun-live-cancellable', 'shell_exec', 'running', {
    cancellable: true,
    detached: true,
    elapsed_ms: 8240,
    output_available: true,
    output_stored_bytes: 196,
    output_total_bytes: 196,
  }),
  runFixture('toolrun-live-foreground', 'browser_open', 'running', {
    cancellable: false,
    detached: false,
    elapsed_ms: 2120,
  }),
  runFixture('toolrun-live-completed', 'web_research_batch', 'completed', {
    elapsed_ms: 12400,
    finished_at_unix_ms: now - 18000,
    output_available: true,
    output_stored_bytes: 3480,
    output_total_bytes: 4096,
    output_capped: true,
  }),
  runFixture('toolrun-live-failed', 'file_read', 'failed', {
    elapsed_ms: 330,
    finished_at_unix_ms: now - 12000,
    is_error: true,
  }),
  runFixture('toolrun-live-interrupted', 'ssh_exec', 'interrupted', {
    elapsed_ms: 90200,
    finished_at_unix_ms: now - 6000,
    is_error: true,
    output_available: true,
    output_stored_bytes: 512,
    output_total_bytes: 512,
    output_redacted: true,
  }),
];
let runs = cloneRuns();
let cancellationCount = 0;

const testModule = [
  "import { h, render } from '/assets/app/vendor/preact.module.js';",
  "import htm from '/assets/app/vendor/htm.module.js';",
  "import { Shell } from '/assets/app/components/Shell.js';",
  'const html = htm.bind(h);',
  'window.__tailEscaped = true;',
  'const child = html`<main id="fixture-content">Captain Control fixture</main>`;',
  'render(html`<${Shell} route="chat" children=${child} />`, document.getElementById("app"));',
].join('\n');

const appHtml = `<!doctype html>
<html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Captain Live Runs smoke</title>
<link rel="stylesheet" href="/theme.css"><link rel="stylesheet" href="/app.css">
</head><body data-theme="dark"><div id="app"></div>
<script type="module" src="/test-main.js"></script></body></html>`;

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

function runFixture(runId, toolName, status, overrides = {}) {
  return {
    run_id: runId,
    tool_name: toolName,
    status,
    detached: false,
    cancellable: false,
    started_at_unix_ms: now - 25000,
    finished_at_unix_ms: null,
    elapsed_ms: 25000,
    caller_agent_id: 'captain',
    origin_tool_use_id: 'call-operator-safe',
    input_sha256: 'a'.repeat(64),
    retry_of_run_id: null,
    retry_attempt: 0,
    is_error: false,
    result_available: false,
    result_truncated: false,
    output_available: false,
    output_stored_bytes: null,
    output_total_bytes: null,
    output_sha256: null,
    output_capped: false,
    output_redacted: false,
    ...overrides,
  };
}

function cloneRuns() {
  return baseRuns.map((run) => ({ ...run }));
}

function contentType(path) {
  switch (extname(path)) {
    case '.js':
    case '.mjs': return 'text/javascript; charset=utf-8';
    case '.css': return 'text/css; charset=utf-8';
    case '.png': return 'image/png';
    default: return 'application/octet-stream';
  }
}

function send(response, status, type, body, headers = {}) {
  response.writeHead(status, {
    'content-type': type,
    'cache-control': 'no-store',
    ...headers,
  });
  response.end(body);
}

function json(response, value, status = 200) {
  send(response, status, 'application/json', JSON.stringify(value), {
    'content-security-policy': appCsp,
    'x-frame-options': 'DENY',
  });
}

function assetTarget(pathname) {
  const vendor = new Map([
    ['/assets/app/vendor/preact.module.js', 'vendor/preact/preact.module.js'],
    ['/assets/app/vendor/hooks.module.js', 'vendor/preact/hooks.module.js'],
    ['/assets/app/vendor/htm.module.js', 'vendor/preact/htm.module.js'],
  ]);
  if (vendor.has(pathname)) return resolve(staticRoot, vendor.get(pathname));
  if (pathname === '/assets/logo.png') return resolve(staticRoot, 'logo.png');
  if (!pathname.startsWith('/assets/app/')) return null;
  const relative = normalize(decodeURIComponent(pathname.slice('/assets/app/'.length)));
  const target = resolve(staticRoot, 'js/app', relative);
  if (relative.startsWith('..') || !target.startsWith(resolve(staticRoot, 'js/app'))) return null;
  return target;
}

function tailFor(runId) {
  const content = runId === 'toolrun-live-cancellable'
    ? 'progress 88%\n[REDACTED]\n<img src=x onerror="window.__tailEscaped=false">\nready'
    : `${runId}\noperator-safe output`;
  return {
    run_id: runId,
    status: runs.find((run) => run.run_id === runId)?.status || 'completed',
    start_line: 1,
    end_line: content.split('\n').length,
    total_lines: content.split('\n').length,
    content,
    content_bytes: Buffer.byteLength(content),
    content_truncated: runId === 'toolrun-live-interrupted',
    content_withheld: false,
    sanitized: true,
  };
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url || '/', 'http://127.0.0.1');
  const { pathname } = url;
  if (pathname === '/') {
    return send(response, 200, 'text/html; charset=utf-8', appHtml, {
      'content-security-policy': appCsp,
      'x-frame-options': 'DENY',
    });
  }
  if (pathname === '/test-main.js') {
    return send(response, 200, 'text/javascript; charset=utf-8', testModule, {
      'content-security-policy': appCsp,
    });
  }
  if (pathname === '/theme.css' || pathname === '/app.css') {
    return send(
      response,
      200,
      'text/css; charset=utf-8',
      await readFile(join(staticRoot, 'css', pathname.slice(1))),
    );
  }
  if (pathname === '/api/agents') {
    return json(response, { agents: [{ id: 'captain', name: 'captain' }] });
  }
  if (pathname === '/api/status') {
    return json(response, {
      version: 'v0.1.0-alpha.13',
      artifacts: { artifacts: 0 },
      tool_runs: { running: runs.filter((run) => run.status === 'running').length },
    });
  }
  if (pathname === '/api/models/updates') return json(response, { pending: [], agents: [] });
  if (pathname === '/api/approvals') return json(response, { approvals: [] });
  if (pathname === '/api/agents/captain/sessions') return json(response, { sessions: [] });
  if (pathname === '/api/artifacts') {
    return json(response, {
      items: [],
      status: {
        healthy: true,
        artifacts: 0,
        versions: 0,
        bytes: 0,
        invalid_entries: 0,
        recovered_staging_entries: 0,
        max_artifact_bytes: 52428800,
        max_total_bytes: 536870912,
      },
    });
  }
  if (pathname === '/api/tool-runs' && request.method === 'GET') {
    return json(response, { count: runs.length, items: runs });
  }
  const tailMatch = pathname.match(/^\/api\/tool-runs\/(toolrun-[a-z0-9-]+)\/tail$/);
  if (tailMatch && request.method === 'GET') {
    return json(response, { tail: tailFor(tailMatch[1]) });
  }
  const cancelMatch = pathname.match(/^\/api\/tool-runs\/(toolrun-[a-z0-9-]+)\/cancel$/);
  if (cancelMatch && request.method === 'POST') {
    const run = runs.find((candidate) => candidate.run_id === cancelMatch[1]);
    if (!run || run.status !== 'running' || !run.cancellable) {
      return json(response, { error: 'not cancellable' }, 409);
    }
    cancellationCount += 1;
    Object.assign(run, {
      status: 'cancelled',
      cancellable: false,
      finished_at_unix_ms: Date.now(),
      elapsed_ms: Date.now() - run.started_at_unix_ms,
    });
    return json(response, { status: 'cancelled', run });
  }
  const target = assetTarget(pathname);
  if (target) {
    try {
      return send(response, 200, contentType(target), await readFile(target), {
        'content-security-policy': appCsp,
      });
    } catch {
      return send(response, 404, 'text/plain', 'not found');
    }
  }
  return send(response, 404, 'text/plain', 'not found');
});

await new Promise((resolveListen, rejectListen) => {
  server.once('error', rejectListen);
  server.listen(0, '127.0.0.1', resolveListen);
});

let browser;
try {
  const { chromium } = await importPlaywright();
  browser = await chromium.launch({ headless: true });
  for (const scenario of [
    { name: 'desktop', width: 1280, height: 800 },
    { name: 'zfold6', width: 344, height: 882 },
  ]) {
    runs = cloneRuns();
    const cancelledBefore = cancellationCount;
    const page = await browser.newPage({ viewport: scenario });
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    page.on('dialog', (dialog) => dialog.accept());
    await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'domcontentloaded' });
    await page.locator('.live-runs-trigger').waitFor();
    await page.locator('.live-runs-trigger-count').waitFor();
    assert.equal(await page.locator('.live-runs-trigger-count').innerText(), '2');

    await page.locator('.live-runs-trigger').click();
    await page.locator('.live-runs-drawer').waitFor();
    await page.waitForFunction(() => document.querySelectorAll('.live-run-row').length === 5);
    await page.locator('.live-run-tail pre').waitFor();
    assert.match(await page.locator('.live-run-tail pre').innerText(), /\[REDACTED\]/);
    assert.match(await page.locator('.live-run-tail pre').innerText(), /<img src=x onerror=/);
    assert.equal(await page.evaluate(() => window.__tailEscaped), true, 'tail content executed as HTML');
    assert.equal((await page.locator('body').innerText()).includes(forbiddenSecret), false);

    await page.getByRole('button', { name: 'Échecs' }).click();
    await page.waitForFunction(() => document.querySelectorAll('.live-run-row').length === 1);
    assert.match(await page.locator('.live-run-row').innerText(), /file_read/);
    assert.match(await page.locator('.live-run-status').innerText(), /Échec/i);

    await page.getByRole('button', { name: 'En cours' }).click();
    await page.waitForFunction(() => document.querySelectorAll('.live-run-row').length === 2);
    await page.locator('.live-run-row').nth(1).click();
    await page.locator('.live-run-noncancellable').waitFor();
    assert.equal(await page.locator('.live-run-cancel').count(), 0);

    await page.locator('.live-run-row').first().click();
    await page.locator('.live-run-cancel').click();
    await page.waitForFunction(() => document.querySelectorAll('.live-run-row').length === 1);
    await page.waitForFunction(() => document.querySelector('.live-runs-trigger-count')?.textContent === '1');
    assert.equal(cancellationCount, cancelledBefore + 1);

    await page.getByRole('button', { name: 'Toutes' }).click();
    await page.waitForFunction(() => document.querySelectorAll('.live-run-row').length === 5);
    const cancelledRow = page.locator('.live-run-row', { hasText: 'shell_exec' });
    assert.match(await cancelledRow.innerText(), /Annulée/);

    await page.getByRole('button', { name: 'Fermer les exécutions' }).click();
    await page.waitForFunction(() => !document.querySelector('.live-runs-drawer'));
    await page.locator('.artifact-trigger').click();
    await page.locator('.artifact-drawer').waitFor();
    assert.equal(await page.locator('.live-runs-drawer').count(), 0);
    await page.getByRole('button', { name: 'Fermer les fichiers' }).click();
    await page.waitForFunction(() => !document.querySelector('.artifact-drawer'));
    await page.locator('.live-runs-trigger').click();
    await page.locator('.live-runs-drawer').waitFor();
    assert.equal(await page.locator('.artifact-drawer').count(), 0);
    await page.waitForTimeout(220);

    const layout = await page.evaluate(() => {
      const drawer = document.querySelector('.live-runs-drawer').getBoundingClientRect();
      const header = document.querySelector('.artifact-header').getBoundingClientRect();
      const summary = document.querySelector('.live-runs-summary').getBoundingClientRect();
      const filters = document.querySelector('.live-runs-filters').getBoundingClientRect();
      const body = document.querySelector('.live-runs-body').getBoundingClientRect();
      return {
        scrollWidth: document.documentElement.scrollWidth,
        scrollHeight: document.documentElement.scrollHeight,
        drawer: [drawer.left, drawer.top, drawer.right, drawer.bottom],
        ordered: summary.top >= header.bottom - 1
          && filters.top >= summary.bottom - 1
          && body.top >= filters.bottom - 1,
      };
    });
    assert.ok(layout.scrollWidth <= scenario.width, `${scenario.name} has horizontal overflow`);
    assert.ok(layout.scrollHeight <= scenario.height, `${scenario.name} has page overflow`);
    assert.ok(
      layout.drawer[0] >= 0 && layout.drawer[2] <= scenario.width + 0.5,
      `${scenario.name} drawer is outside viewport: ${JSON.stringify(layout.drawer)}`,
    );
    assert.ok(
      layout.drawer[1] >= 0 && layout.drawer[3] <= scenario.height + 0.5,
      `${scenario.name} drawer vertical bounds are invalid: ${JSON.stringify(layout.drawer)}`,
    );
    assert.equal(layout.ordered, true, `${scenario.name} header/summary/filter/body overlap`);
    assert.deepEqual(pageErrors, []);

    const screenshot = await page.screenshot({
      path: `/tmp/captain-live-runs-${scenario.name}.png`,
      fullPage: false,
    });
    assert.ok(screenshot.length > 12000, `${scenario.name} screenshot is unexpectedly blank`);
    await page.close();
  }
  process.stdout.write('Control Live Runs smoke passed: authenticated UI contract, filters, redacted tail, strict cancellation, desktop and Z Fold layouts.\n');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
