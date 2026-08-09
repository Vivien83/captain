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
const previewCsp = "sandbox; default-src 'none'; img-src data:; media-src data:; font-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";
const artifactId = '01234567-89ab-cdef-0123-456789abcdef';
const svgId = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee';

const versions = [
  {
    artifact_id: artifactId,
    version: 2,
    agent_id: 'captain',
    session_id: 'session-1',
    title: 'Rapport de déploiement',
    filename: 'deployment-report.html',
    mime_type: 'text/html',
    preview_kind: 'html',
    size_bytes: 18432,
    sha256: 'a'.repeat(64),
    created_at: '2026-08-08T20:15:00Z',
  },
  {
    artifact_id: artifactId,
    version: 1,
    agent_id: 'captain',
    session_id: 'session-1',
    title: 'Rapport de déploiement',
    filename: 'deployment-report.html',
    mime_type: 'text/html',
    preview_kind: 'html',
    size_bytes: 12288,
    sha256: 'b'.repeat(64),
    created_at: '2026-08-08T19:10:00Z',
  },
];
const svgArtifact = {
  artifact_id: svgId,
  version: 1,
  agent_id: 'captain',
  session_id: 'session-1',
  title: 'Schéma actif',
  filename: 'diagram.svg',
  mime_type: 'image/svg+xml',
  preview_kind: 'none',
  size_bytes: 4096,
  sha256: 'c'.repeat(64),
  created_at: '2026-08-08T18:00:00Z',
};

const testModule = [
  "import { h, render } from '/assets/app/vendor/preact.module.js';",
  "import htm from '/assets/app/vendor/htm.module.js';",
  "import { ArtifactDrawer } from '/assets/app/components/ArtifactDrawer.js';",
  'const html = htm.bind(h);',
  'window.__artifactEscaped = true;',
  'window.__artifactCount = null;',
  'render(html`<${ArtifactDrawer} open=${true} onClose=${() => { window.__closed = true; }} onCount=${(count) => { window.__artifactCount = count; }} />`, document.getElementById("app"));',
].join('\n');

const appHtml = `<!doctype html>
<html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Captain artifact smoke</title>
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

function send(response, status, type, body, headers = {}) {
  response.writeHead(status, {
    'content-type': type,
    'cache-control': 'no-store',
    ...headers,
  });
  response.end(body);
}

function json(response, value) {
  send(response, 200, 'application/json', JSON.stringify(value), {
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
  if (!pathname.startsWith('/assets/app/')) return null;
  const relative = normalize(decodeURIComponent(pathname.slice('/assets/app/'.length)));
  const target = resolve(staticRoot, 'js/app', relative);
  if (relative.startsWith('..') || !target.startsWith(resolve(staticRoot, 'js/app'))) return null;
  return target;
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
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
    const target = join(staticRoot, 'css', pathname.slice(1));
    return send(response, 200, 'text/css; charset=utf-8', await readFile(target));
  }
  if (pathname === '/api/artifacts') {
    return json(response, {
      items: [versions[0], svgArtifact],
      status: {
        healthy: true,
        artifacts: 2,
        versions: 3,
        bytes: 34816,
        invalid_entries: 0,
        recovered_staging_entries: 0,
        max_artifact_bytes: 52428800,
        max_total_bytes: 536870912,
      },
    });
  }
  if (pathname === `/api/artifacts/${artifactId}/versions`) {
    return json(response, { artifact_id: artifactId, count: versions.length, items: versions });
  }
  if (pathname === `/api/artifacts/${svgId}/versions`) {
    return json(response, { artifact_id: svgId, count: 1, items: [svgArtifact] });
  }
  const preview = pathname.match(new RegExp(`^/api/artifacts/${artifactId}/versions/(1|2)/preview$`));
  if (preview) {
    const version = preview[1];
    return send(
      response,
      200,
      'text/html; charset=utf-8',
      `<!doctype html><html><body style="margin:20px;font:16px sans-serif"><h1>Rapport v${version}</h1><p>Contenu vérifié.</p><script>parent.__artifactEscaped = false;</script></body></html>`,
      {
        'content-security-policy': previewCsp,
        'x-frame-options': 'SAMEORIGIN',
        'referrer-policy': 'no-referrer',
      },
    );
  }
  if (pathname.includes('/download')) {
    return send(response, 200, 'application/octet-stream', 'verified download', {
      'content-disposition': 'attachment; filename="artifact.bin"',
      'content-security-policy': "sandbox; default-src 'none'; frame-ancestors 'none'",
      'x-frame-options': 'DENY',
    });
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
    const page = await browser.newPage({ viewport: scenario });
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'networkidle' });
    await page.locator('.artifact-row').first().waitFor();
    await page.waitForFunction(() => window.__artifactCount === 2);

    assert.equal(await page.locator('.artifact-row').count(), 2);
    assert.equal(await page.locator('.artifact-drawer').getAttribute('role'), 'dialog');
    assert.equal(await page.locator('.artifact-preview iframe').getAttribute('sandbox'), '');
    assert.match(await page.frameLocator('.artifact-preview iframe').locator('body').innerText(), /Rapport v2/);
    assert.equal(await page.evaluate(() => window.__artifactEscaped), true, 'sandboxed HTML executed script');

    await page.locator('.artifact-row').nth(1).click();
    await page.locator('.artifact-preview-empty').waitFor();
    assert.match(await page.locator('.artifact-preview-empty').innerText(), /Aperçu indisponible/);
    assert.equal(await page.locator('.artifact-preview iframe').count(), 0);

    await page.locator('.artifact-row').first().click();
    await page.locator('.artifact-detail-bar select').selectOption('1');
    await page.waitForFunction(() => document.querySelector('.artifact-preview iframe')?.src.includes('/versions/1/preview'));
    assert.match(await page.locator('.artifact-row.active strong').innerText(), /Rapport de déploiement/);
    assert.match(await page.frameLocator('.artifact-preview iframe').locator('body').innerText(), /Rapport v1/);
    assert.match(await page.locator('.artifact-icon-action').getAttribute('href'), /\/versions\/1\/download$/);

    const layout = await page.evaluate(() => {
      const drawer = document.querySelector('.artifact-drawer').getBoundingClientRect();
      const header = document.querySelector('.artifact-header').getBoundingClientRect();
      const summary = document.querySelector('.artifact-summary').getBoundingClientRect();
      const body = document.querySelector('.artifact-body').getBoundingClientRect();
      return {
        viewport: [innerWidth, innerHeight],
        scrollWidth: document.documentElement.scrollWidth,
        scrollHeight: document.documentElement.scrollHeight,
        drawer: [drawer.left, drawer.top, drawer.right, drawer.bottom],
        ordered: summary.top >= header.bottom - 1 && body.top >= summary.bottom - 1,
      };
    });
    assert.ok(layout.scrollWidth <= scenario.width, `${scenario.name} has horizontal overflow`);
    assert.ok(layout.scrollHeight <= scenario.height, `${scenario.name} has page overflow`);
    assert.ok(layout.drawer[0] >= 0 && layout.drawer[2] <= scenario.width + 0.5);
    assert.ok(layout.drawer[1] >= 0 && layout.drawer[3] <= scenario.height + 0.5);
    assert.equal(layout.ordered, true, `${scenario.name} header/summary/body overlap`);
    assert.deepEqual(pageErrors, []);

    const screenshot = await page.screenshot({
      path: `/tmp/captain-artifact-drawer-${scenario.name}.png`,
      fullPage: false,
    });
    assert.ok(screenshot.length > 12000, `${scenario.name} screenshot is unexpectedly blank`);
    await page.close();
  }
  process.stdout.write('Control artifact drawer smoke passed: desktop and Z Fold layouts, sandbox preview, versions, and download links.\n');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
