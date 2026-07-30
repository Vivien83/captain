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
  <title>Captain project completion smoke</title>
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
  import { ProjectRuntime } from '/static/js/app/views/ProjectRuntime.js';
  const html = htm.bind(h);
  const proof = (decision, evidence_count) => ({
    protocol: 'captain.completion.v1', decision, evidence_count,
  });
  const runtime = {
    status: 'blocked', current_phase: 'verify', progress: 84,
    manager_agent: { name: 'captain', model: 'gpt-5.6-sol' },
    parallelism: { running: 0, max_parallel_agents: 4 },
    completion_contract: proof('insufficient_evidence', 8),
    workers: [
      { id: 'observer', role: 'observer', phase: 'observe', status: 'done',
        summary: 'Repository state and constraints captured.',
        completion_contract: proof('satisfied', 3) },
      { id: 'builder', role: 'builder', phase: 'build', status: 'done',
        summary: 'Focused implementation completed without modifying unrelated user work.',
        completion_contract: proof('satisfied', 4) },
      { id: 'verifier', role: 'verifier', phase: 'verify', status: 'blocked',
        summary: 'Completion claim rejected because the independent smoke receipt is missing.',
        completion_contract: proof('insufficient_evidence', 1) },
    ],
    timeline: [
      { id: 'e1', title: 'Verifier completion rejected',
        detail: 'Captain preserved the phase for review instead of marking the project complete.',
        actor: 'captain', phase: 'verify', status: 'blocked' },
      { id: 'e2', title: 'Builder completed',
        detail: 'Four hashed execution receipts were recorded.',
        actor: 'builder', phase: 'build', status: 'done' },
    ],
  };
  function Preview() {
    return html\`<main class="page"><div class="page-inner page-inner-wide">
      <div class="page-heading"><div><h1 class="page-title">Runtime projet</h1>
      <p class="page-sub">Contrats de fin fondés sur des preuves</p></div></div>
      <\${ProjectRuntime} projectId="demo" runtime=\${runtime} operatorStatus=\${null}
        onRefresh=\${async () => {}} />
    </div></main>\`;
  }
  render(html\`<\${Preview} />\`, document.getElementById('app'));
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
    case '.png': return 'image/png';
    case '.svg': return 'image/svg+xml';
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
    await page.getByText('preuves insuffisantes · 8 reçu(s)').waitFor();
    const snapshot = await page.evaluate(() => ({
      text: document.querySelector('.page-inner')?.textContent || '',
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
      overflowing: [...document.querySelectorAll('body *')]
        .filter((element) => {
          const rect = element.getBoundingClientRect();
          return rect.left < -0.5 || rect.right > document.documentElement.clientWidth + 0.5;
        })
        .map((element) => `${element.tagName}.${element.className}`)
        .slice(0, 20),
    }));
    assert.equal(pageErrors.length, 0, `${surface.name}: ${pageErrors.join('; ')}`);
    assert.equal(snapshot.scrollWidth, snapshot.clientWidth, `${surface.name}: horizontal overflow`);
    assert.deepEqual(snapshot.overflowing, [], `${surface.name}: elements leave viewport`);
    assert.match(snapshot.text, /preuves validées · 3 reçu\(s\)/);
    assert.match(snapshot.text, /preuves insuffisantes · 1 reçu\(s\)/);
    await page.screenshot({
      path: `/private/tmp/captain-project-completion-${surface.name}.png`,
      fullPage: true,
    });
    await page.close();
  }
  console.log('project completion Control/Desktop surfaces smoke: PASS');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
