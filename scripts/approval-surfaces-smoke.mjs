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

const appAssetAliases = new Map([
  ['vendor/preact.module.js', join(staticRoot, 'vendor/preact/preact.module.js')],
  ['vendor/hooks.module.js', join(staticRoot, 'vendor/preact/hooks.module.js')],
  ['vendor/htm.module.js', join(staticRoot, 'vendor/preact/htm.module.js')],
  ['vendor/marked.esm.js', join(staticRoot, 'vendor/marked/marked.esm.js')],
  ['vendor/purify.es.mjs', join(staticRoot, 'vendor/dompurify/purify.es.mjs')],
]);

let pending = [];
let rules = [];
let lastDurableDeny = null;

function resetFixture() {
  pending = [{
    id: 'pending-1',
    agent_id: 'captain',
    agent_name: 'captain',
    tool_name: 'shell_exec',
    action_summary: 'Déployer la version vérifiée sur production',
    risk_level: 'critical',
    requested_at: new Date().toISOString(),
  }];
  rules = [{
    id: 'rule-1',
    effect: 'allow',
    agent_id: 'captain',
    tool_name: 'file_write',
    action_digest: 'a'.repeat(64),
    reason: null,
  }];
  lastDurableDeny = null;
}

function send(response, status, type, body) {
  response.writeHead(status, { 'content-type': type, 'cache-control': 'no-store' });
  response.end(body);
}

async function jsonBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (pathname === '/assets/logo.png') return send(response, 200, 'image/png', logo);
  if (pathname === '/api/auth/check') {
    return send(response, 200, 'application/json', JSON.stringify({ mode: 'session', authenticated: true }));
  }
  if (pathname === '/api/agents') {
    return send(response, 200, 'application/json', JSON.stringify([
      { id: 'captain', name: 'captain', model_provider: 'codex', model_name: 'gpt-5.6-sol' },
    ]));
  }
  if (pathname === '/api/status') {
    return send(response, 200, 'application/json', JSON.stringify({ version: '0.1.0-alpha.9' }));
  }
  if (pathname === '/api/models/updates') {
    return send(response, 200, 'application/json', JSON.stringify({ pending: [], agents: [] }));
  }
  if (pathname === '/api/approvals' && request.method === 'GET') {
    return send(response, 200, 'application/json', JSON.stringify({
      approvals: pending,
      total: pending.length,
      rules,
      rules_total: rules.length,
    }));
  }
  if (pathname === '/api/approvals/pending-1/reject_always' && request.method === 'POST') {
    lastDurableDeny = await jsonBody(request);
    pending = [];
    rules.push({
      id: 'rule-2',
      effect: 'deny',
      agent_id: 'captain',
      tool_name: 'shell_exec',
      action_digest: 'b'.repeat(64),
      reason: lastDurableDeny.reason,
    });
    return send(response, 200, 'application/json', JSON.stringify({ status: 'rejected_always' }));
  }
  if (pathname === '/api/approvals/rules/rule-1' && request.method === 'DELETE') {
    rules = rules.filter((rule) => rule.id !== 'rule-1');
    return send(response, 200, 'application/json', JSON.stringify({ status: 'revoked' }));
  }
  if (pathname.startsWith('/assets/app/')) {
    const relative = normalize(pathname.slice('/assets/app/'.length));
    if (relative.startsWith('..')) return send(response, 404, 'text/plain', 'not found');
    try {
      const content = await readFile(appAssetAliases.get(relative) || join(staticRoot, 'js/app', relative));
      const type = ['.mjs', '.js'].includes(extname(relative))
        ? 'text/javascript; charset=utf-8'
        : 'application/octet-stream';
      return send(response, 200, type, content);
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
  const playwright = await importPlaywright();
  browser = await playwright.chromium.launch({ headless: true });
  const port = server.address().port;
  for (const surface of [
    { name: 'desktop', viewport: { width: 1280, height: 800 } },
    { name: 'mobile', viewport: { width: 390, height: 844 } },
  ]) {
    resetFixture();
    const page = await browser.newPage({ viewport: surface.viewport });
    await page.goto(`http://127.0.0.1:${port}/#/approvals`, { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.approval-card');
    await page.waitForSelector('.approval-rule');

    const snapshot = await page.evaluate(() => {
      const card = document.querySelector('.approval-card').getBoundingClientRect();
      const rulesSection = document.querySelector('.approval-rules').getBoundingClientRect();
      return {
        text: document.querySelector('.page-inner').textContent,
        bodyOverflow: document.documentElement.scrollWidth - window.innerWidth,
        cardRight: card.right,
        rulesRight: rulesSection.right,
        viewportWidth: window.innerWidth,
        buttonCount: document.querySelectorAll('.approval-card button').length,
      };
    });
    assert.match(snapshot.text, /Toujours cette action/);
    assert.match(snapshot.text, /Bloquer cette action/);
    assert.match(snapshot.text, /Règles durables/);
    assert.match(snapshot.text, /empreinte exacte/);
    assert.equal(snapshot.buttonCount, 6);
    assert.ok(snapshot.bodyOverflow <= 1, `${surface.name}: page overflow ${snapshot.bodyOverflow}px`);
    assert.ok(snapshot.cardRight <= snapshot.viewportWidth + 1, `${surface.name}: approval card leaves viewport`);
    assert.ok(snapshot.rulesRight <= snapshot.viewportWidth + 1, `${surface.name}: rules leave viewport`);

    const durableDeny = page.getByRole('button', { name: 'Bloquer cette action' });
    assert.equal(await durableDeny.isDisabled(), true);
    await page.getByLabel('Motif transmis à l’agent').fill('Utilise le serveur de test');
    assert.equal(await durableDeny.isEnabled(), true);

    await page.screenshot({
      path: `/private/tmp/captain-approvals-${surface.name}.png`,
      fullPage: true,
    });

    await durableDeny.click();
    await page.waitForFunction(() => document.querySelectorAll('.approval-card').length === 0);
    assert.deepEqual(lastDurableDeny, { reason: 'Utilise le serveur de test' });
    assert.equal(await page.locator('.approval-rule').count(), 2);
    await page.getByRole('button', { name: 'Révoquer' }).first().click();
    await page.waitForFunction(() => document.querySelectorAll('.approval-rule').length === 1);
    await page.close();
  }
  console.log('approval Control/Desktop surfaces smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
