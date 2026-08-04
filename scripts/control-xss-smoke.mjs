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
const cspMatch = middlewareSource.match(
  /CONTENT_SECURITY_POLICY: &str = "([^"]+)"/,
);
assert.ok(cspMatch, 'the production CSP constant must remain statically inspectable');
const contentSecurityPolicy = cspMatch[1];
assert.doesNotMatch(contentSecurityPolicy, /unsafe-eval/);
assert.doesNotMatch(
  contentSecurityPolicy.split(';').find((item) => item.trim().startsWith('script-src ')) || '',
  /unsafe-inline/,
);

const controlVendorAliases = new Map([
  ['/assets/app/vendor/preact.module.js', 'vendor/preact/preact.module.js'],
  ['/assets/app/vendor/hooks.module.js', 'vendor/preact/hooks.module.js'],
  ['/assets/app/vendor/htm.module.js', 'vendor/preact/htm.module.js'],
  ['/assets/app/vendor/marked.esm.js', 'vendor/marked/marked.esm.js'],
  ['/assets/app/vendor/purify.es.mjs', 'vendor/dompurify/purify.es.mjs'],
]);

const markdownPayload = [
  '# Safe heading',
  '',
  'Visible **Markdown** remains.',
  '',
  '<img src="/xss-probe" onerror="window.__captainXss += 1">',
  '<script>window.__captainXss += 2</script>',
  '<form><input autofocus onfocus="window.__captainXss += 4"></form>',
  '<svg onload="window.__captainXss += 8"><circle></circle></svg>',
  '<a href="javascript:window.__captainXss+=16">unsafe link</a>',
  '<a href="data:text/html,unsafe">data link</a>',
].join('\n');
const toolPayload = '<img src="/tool-xss" onerror="window.__captainXss += 32">';
const sessionPayload = '<svg onload="window.__captainXss += 64">session</svg>';

const testModule = [
  "import { h, render } from '/assets/app/vendor/preact.module.js';",
  "import htm from '/assets/app/vendor/htm.module.js';",
  "import { Markdown } from '/static/js/app/components/Markdown.js';",
  "import { ToolCard } from '/static/js/app/components/ToolCard.js';",
  "import { SessionRow } from '/static/js/app/components/Shell.js';",
  'const html = htm.bind(h);',
  'window.__captainXss = 0;',
  'window.__captainCspViolations = [];',
  "document.addEventListener('securitypolicyviolation', (event) => {",
  '  window.__captainCspViolations.push(event.violatedDirective);',
  '});',
  `const markdownPayload = ${JSON.stringify(markdownPayload)};`,
  `const toolPayload = ${JSON.stringify(toolPayload)};`,
  `const sessionPayload = ${JSON.stringify(sessionPayload)};`,
  'const tool = {',
  "  id: 'tool-xss', name: toolPayload, input: toolPayload, result: toolPayload,",
  '  done: true, isError: false, startedAt: 1, endedAt: 2,',
  '};',
  'const session = { session_id: "session-xss", label: sessionPayload };',
  'render(html`',
  '  <main>',
  '    <section id="markdown-probe"><${Markdown} text=${markdownPayload} /></section>',
  '    <section id="tool-probe"><${ToolCard} tool=${tool} /></section>',
  '    <section id="session-probe"><${SessionRow} session=${session} agentId="captain" active=${false} /></section>',
  '  </main>',
  "`, document.getElementById('app'));",
  'window.__captainRendered = true;',
].join('\n');

const appHtml = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>Captain Control XSS smoke</title>
</head>
<body><div id="app"></div><script type="module" src="/test-main.js"></script></body>
</html>`;

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
    default: return 'application/octet-stream';
  }
}

function send(response, status, type, body) {
  response.writeHead(status, {
    'content-type': type,
    'content-security-policy': contentSecurityPolicy,
    'cache-control': 'no-store',
  });
  response.end(body);
}

const server = createServer(async (request, response) => {
  const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname;
  if (pathname === '/') return send(response, 200, 'text/html; charset=utf-8', appHtml);
  if (pathname === '/test-main.js') {
    return send(response, 200, 'text/javascript; charset=utf-8', testModule);
  }
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
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const pageErrors = [];
  page.on('pageerror', (error) => pageErrors.push(error.message));

  const response = await page.goto(`http://127.0.0.1:${server.address().port}`, {
    waitUntil: 'networkidle',
  });
  assert.equal(response.headers()['content-security-policy'], contentSecurityPolicy);
  await page.waitForFunction(() => window.__captainRendered === true);
  await page.locator('#tool-probe .head').click();

  const result = await page.evaluate(() => {
    const markdown = document.querySelector('#markdown-probe');
    const unsafeLink = Array.from(markdown.querySelectorAll('a'))
      .find((link) => link.textContent.includes('unsafe link'));
    const dataLink = Array.from(markdown.querySelectorAll('a'))
      .find((link) => link.textContent.includes('data link'));
    return {
      xss: window.__captainXss,
      cspViolations: window.__captainCspViolations,
      markdownText: markdown.textContent,
      markdownActiveNodes: markdown.querySelectorAll('script, form, input, svg, style, iframe, object, embed').length,
      unsafeHref: unsafeLink && unsafeLink.getAttribute('href'),
      dataHref: dataLink && dataLink.getAttribute('href'),
      linkTarget: unsafeLink && unsafeLink.getAttribute('target'),
      linkRel: unsafeLink && unsafeLink.getAttribute('rel'),
      toolName: document.querySelector('#tool-probe .tool-name').textContent,
      toolBody: document.querySelector('#tool-probe .body').textContent,
      toolInjectedNodes: document.querySelectorAll('#tool-probe img, #tool-probe svg, #tool-probe script').length,
      sessionLabel: document.querySelector('#session-probe .label').textContent,
      sessionInjectedNodes: document.querySelectorAll('#session-probe img, #session-probe svg, #session-probe script').length,
    };
  });

  assert.deepEqual(pageErrors, []);
  assert.equal(result.xss, 0, 'attacker-controlled UI content executed JavaScript');
  assert.deepEqual(result.cspViolations, []);
  assert.match(result.markdownText, /Visible Markdown remains/);
  assert.equal(result.markdownActiveNodes, 0);
  assert.equal(result.unsafeHref, null);
  assert.equal(result.dataHref, null);
  assert.equal(result.linkTarget, '_blank');
  assert.equal(result.linkRel, 'noopener noreferrer');
  assert.equal(result.toolName, toolPayload);
  assert.match(result.toolBody, /tool-xss/);
  assert.equal(result.toolInjectedNodes, 0);
  assert.equal(result.sessionLabel, sessionPayload);
  assert.equal(result.sessionInjectedNodes, 0);
  await page.close();

  process.stdout.write('Control XSS smoke passed: CSP, Markdown, tool output, and session labels stay inert.\n');
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
