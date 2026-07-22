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

const [themeCss, appCss, appBody, logo] = await Promise.all([
  readFile(join(staticRoot, 'css/theme.css'), 'utf8'),
  readFile(join(staticRoot, 'css/app.css'), 'utf8'),
  readFile(join(staticRoot, 'app_body.html'), 'utf8'),
  readFile(join(repoRoot, 'assets/logo.png')),
]);
const appHtml = `<!doctype html><html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><style>${themeCss}\n${appCss}</style>
</head><body data-theme="dark">${appBody}</body></html>`;

const budget = {
  provider_subscriptions: {
    state: 'warning',
    reported_by_provider: true,
    items: [
      {
        provider: 'codex',
        limit_id: 'codex',
        limit_name: 'Codex',
        plan_type: 'pro',
        alert_level: 'warning',
        stale: false,
        primary: {
          used_percent: 72.5,
          window_seconds: 18000,
          reset_after_seconds: 3600,
        },
        secondary: {
          used_percent: 41,
          window_seconds: 604800,
          reset_after_seconds: 86400,
        },
        credits: { has_credits: true, unlimited: false, balance: '17.50' },
      },
      {
        provider: 'codex',
        limit_id: 'codex_bengalfox',
        limit_name: 'GPT-5.3-Codex-Spark',
        plan_type: 'pro',
        alert_level: 'normal',
        stale: false,
        primary: {
          used_percent: 5,
          window_seconds: 604800,
          reset_after_seconds: 86400,
        },
      },
    ],
  },
};

const jsonRoutes = new Map([
  ['/api/auth/check', { mode: 'session', authenticated: true }],
  ['/api/agents', [{ id: 'captain', name: 'captain', model_provider: 'codex', model_name: 'gpt-5.6-sol' }]],
  ['/api/status', { version: '0.1.0-alpha.9' }],
  ['/api/models/updates', { pending: [], agents: [] }],
  ['/api/approvals', { approvals: [] }],
  ['/api/agents/captain/sessions', { sessions: [] }],
  ['/api/budget', budget],
]);
const appAssetAliases = new Map([
  ['vendor/preact.module.js', join(staticRoot, 'vendor/preact/preact.module.js')],
  ['vendor/hooks.module.js', join(staticRoot, 'vendor/preact/hooks.module.js')],
  ['vendor/htm.module.js', join(staticRoot, 'vendor/preact/htm.module.js')],
  ['vendor/marked.esm.js', join(staticRoot, 'vendor/marked/marked.esm.js')],
  ['vendor/purify.es.mjs', join(staticRoot, 'vendor/dompurify/purify.es.mjs')],
]);

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (pathname === '/assets/logo.png') return send(response, 200, 'image/png', logo);
  if (jsonRoutes.has(pathname)) {
    return send(response, 200, 'application/json', JSON.stringify(jsonRoutes.get(pathname)));
  }
  if (pathname.startsWith('/assets/app/')) {
    const relative = normalize(pathname.slice('/assets/app/'.length));
    if (relative.startsWith('..')) return send(response, 404, 'text/plain', 'not found');
    try {
      const content = await readFile(appAssetAliases.get(relative) || join(staticRoot, 'js/app', relative));
      const type = extname(relative) === '.mjs' || extname(relative) === '.js'
        ? 'text/javascript; charset=utf-8'
        : 'application/octet-stream';
      return send(response, 200, type, content);
    } catch {
      return send(response, 404, 'text/plain', 'not found');
    }
  }
  return send(response, 404, 'text/plain', 'not found');
});

function send(response, status, type, body) {
  response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  response.end(body);
}

await new Promise((resolveListen, rejectListen) => {
  server.once('error', rejectListen);
  server.listen(0, '127.0.0.1', resolveListen);
});

let browser;
try {
  const playwright = await importPlaywright();
  browser = await playwright.chromium.launch({ headless: true });
  const port = server.address().port;
  for (const surface of [
    { name: 'desktop', viewport: { width: 1280, height: 800 } },
    { name: 'mobile', viewport: { width: 390, height: 844 } },
  ]) {
    const page = await browser.newPage({ viewport: surface.viewport });
    await page.goto(`http://127.0.0.1:${port}/`, { waitUntil: 'networkidle' });
    await page.waitForSelector('.provider-quota-window');
    const snapshot = await page.evaluate(() => {
      const windows = [...document.querySelectorAll('.provider-quota-window')];
      const bar = document.querySelector('.provider-quota-bar').getBoundingClientRect();
      const composer = document.querySelector('.composer-wrap').getBoundingClientRect();
      return {
        windowCount: windows.length,
        progressValues: windows.map((item) => Number(item.querySelector('[role="progressbar"]').getAttribute('aria-valuenow'))),
        text: document.querySelector('.provider-quota-bar').textContent,
        alternativeText: document.querySelector('.provider-quota-more')?.textContent || '',
        bodyOverflow: document.documentElement.scrollWidth - window.innerWidth,
        overlap: composer.bottom - bar.top,
        barBottom: bar.bottom,
        viewportHeight: window.innerHeight,
      };
    });
    assert.equal(snapshot.windowCount, 2, `${surface.name}: one gauge per provider window`);
    assert.deepEqual(snapshot.progressValues, [72.5, 41]);
    assert.match(snapshot.text, /Codex/);
    assert.match(snapshot.text, /Actif\s*:\s*gpt-5\.6-sol/);
    assert.match(snapshot.text, /17\.50/);
    assert.doesNotMatch(snapshot.text, /GPT-5\.3-Codex-Spark/);
    assert.match(snapshot.alternativeText, /\+1 quota annexe/);
    assert.match(snapshot.alternativeText, /hors modèle actif/);
    assert.ok(snapshot.bodyOverflow <= 1, `${surface.name}: page overflow ${snapshot.bodyOverflow}px`);
    assert.ok(snapshot.overlap <= 1, `${surface.name}: quota bar overlaps composer by ${snapshot.overlap}px`);
    assert.ok(snapshot.barBottom <= snapshot.viewportHeight + 1, `${surface.name}: quota bar leaves viewport`);
    await page.screenshot({
      path: `/private/tmp/captain-provider-quota-${surface.name}.png`,
      fullPage: false,
    });
    await page.close();
  }
  console.log('provider quota terminal-independent surfaces smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
