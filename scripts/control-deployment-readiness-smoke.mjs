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

const statusPayload = {
  status: 'running',
  version: '0.1.0-alpha.13',
  uptime_seconds: 3600,
  default_provider: 'codex',
  default_model: 'gpt-5.6-sol',
  llm_driver_ready: true,
  runtime_health: { state: 'ok', issue_count: 0, issues: [], operator_actions: [] },
  deployment: {
    profile: 'vps',
    public_url: 'https://agent.captain.example',
    https: true,
    reverse_proxy: 'caddy',
    readiness: {
      state: 'degraded',
      checked_at: '2026-08-08T12:00:00Z',
      duration_ms: 284,
      next_check_at: '2026-08-08T12:05:00Z',
      checks: [
        { id: 'local_health', status: 'ok', summary: 'Local Captain health is healthy' },
        { id: 'dns', status: 'ok', summary: 'The public domain resolves safely' },
        {
          id: 'public_health',
          status: 'warning',
          summary: 'Public Captain health is degraded',
          remediation: 'Inspect health detail before production use.',
        },
      ],
      operator_actions: ['Inspect health detail before production use.'],
    },
  },
};

const testModule = [
  "import { h, render } from '/assets/app/vendor/preact.module.js';",
  "import htm from '/assets/app/vendor/htm.module.js';",
  "import { Status } from '/assets/app/views/Status.js';",
  'const html = htm.bind(h);',
  'render(html`<${Status} />`, document.getElementById("app"));',
].join('\n');

const appHtml = `<!doctype html>
<html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Captain deployment readiness smoke</title>
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

function contentType(path) {
  switch (extname(path)) {
    case '.js':
    case '.mjs': return 'text/javascript; charset=utf-8';
    case '.css': return 'text/css; charset=utf-8';
    default: return 'application/octet-stream';
  }
}

function send(response, status, type, body) {
  response.writeHead(status, {
    'content-type': type,
    'cache-control': 'no-store',
    'content-security-policy': appCsp,
    'x-frame-options': 'DENY',
  });
  response.end(body);
}

function assetTarget(pathname) {
  const vendor = new Map([
    ['/assets/app/vendor/preact.module.js', 'vendor/preact/preact.module.js'],
    ['/assets/app/vendor/hooks.module.js', 'vendor/preact/hooks.module.js'],
    ['/assets/app/vendor/htm.module.js', 'vendor/preact/htm.module.js'],
  ]);
  if (vendor.has(pathname)) return resolve(staticRoot, vendor.get(pathname));
  if (!pathname.startsWith('/assets/app/')) return null;
  const relative = normalize(decodeURIComponent(pathname.slice('/assets/app/'.length)));
  const target = resolve(staticRoot, 'js/app', relative);
  if (relative.startsWith('..') || !target.startsWith(resolve(staticRoot, 'js/app'))) return null;
  return target;
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (pathname === '/test-main.js') {
    return send(response, 200, 'text/javascript; charset=utf-8', testModule);
  }
  if (pathname === '/theme.css' || pathname === '/app.css') {
    return send(
      response,
      200,
      'text/css; charset=utf-8',
      await readFile(join(staticRoot, 'css', pathname.slice(1))),
    );
  }
  if (pathname === '/api/status') {
    return send(response, 200, 'application/json', JSON.stringify(statusPayload));
  }
  const target = assetTarget(pathname);
  if (target) {
    try {
      return send(response, 200, contentType(target), await readFile(target));
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
    { name: 'desktop', width: 1280, height: 900 },
    { name: 'zfold6', width: 344, height: 882 },
  ]) {
    const page = await browser.newPage({ viewport: scenario });
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'networkidle' });
    const deploymentHeading = page.getByRole('heading', { name: 'Déploiement' });
    await deploymentHeading.waitFor();

    assert.equal(await page.getByText('https://agent.captain.example', { exact: true }).count(), 1);
    assert.equal(await page.getByText('Public Captain health is degraded', { exact: true }).count(), 1);
    assert.equal(await page.getByText('Inspect health detail before production use.', { exact: true }).count(), 1);
    assert.deepEqual(pageErrors, []);

    const layout = await page.evaluate(() => ({
      width: innerWidth,
      scrollWidth: document.documentElement.scrollWidth,
      deploymentTop: [...document.querySelectorAll('.status-section-head h2')]
        .find((node) => node.textContent === 'Déploiement')
        ?.getBoundingClientRect().top,
      cellsContained: [...document.querySelectorAll('.status-cell')].every((cell) => {
        const box = cell.getBoundingClientRect();
        return box.left >= -0.5 && box.right <= innerWidth + 0.5;
      }),
    }));
    assert.ok(layout.scrollWidth <= scenario.width, `${scenario.name} has horizontal overflow`);
    assert.ok(Number.isFinite(layout.deploymentTop), `${scenario.name} deployment section is missing`);
    assert.equal(layout.cellsContained, true, `${scenario.name} status cells escape the viewport`);

    const screenshot = await page.screenshot({
      path: `/tmp/captain-deployment-readiness-${scenario.name}.png`,
      fullPage: true,
    });
    assert.ok(screenshot.length > 12000, `${scenario.name} screenshot is unexpectedly blank`);
    await page.close();
  }
  process.stdout.write('Control deployment readiness smoke passed: desktop and Z Fold layouts, real Status component, CSP, checks and actions.\n');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
