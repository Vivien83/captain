#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const vendorRoot = join(repoRoot, "crates/captain-api/static/vendor/xterm");

async function importPlaywright() {
  const candidates = [];
  if (process.env.CAPTAIN_PLAYWRIGHT_MODULE) {
    candidates.push(process.env.CAPTAIN_PLAYWRIGHT_MODULE);
  }
  candidates.push("playwright");

  try {
    const npmRoot = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
    candidates.push(join(npmRoot, "playwright/index.mjs"));
    candidates.push(join(npmRoot, "playwright/index.js"));
  } catch {
    // A repository-local Playwright install can satisfy the first candidate.
  }

  const errors = [];
  for (const candidate of candidates) {
    try {
      const specifier = candidate.startsWith("/") ? pathToFileURL(candidate).href : candidate;
      return await import(specifier);
    } catch (error) {
      errors.push(`${candidate}: ${error.message}`);
    }
  }
  throw new Error(`Playwright is required. Tried:\n${errors.join("\n")}`);
}

const [xtermJs, unicode11Js, xtermCss] = await Promise.all([
  readFile(join(vendorRoot, "xterm.js")),
  readFile(join(vendorRoot, "addon-unicode11.js")),
  readFile(join(vendorRoot, "xterm.css")),
]);

const testPage = `<!doctype html>
<html><head><meta charset="utf-8"><link rel="stylesheet" href="/xterm.css"></head>
<body><div id="terminal"></div>
<script src="/xterm.js"></script>
<script src="/addon-unicode11.js"></script>
<script>
  const term = new Terminal({ cols: 20, rows: 2, allowProposedApi: true });
  term.loadAddon(new Unicode11Addon.Unicode11Addon());
  term.unicode.activeVersion = '11';
  term.open(document.getElementById('terminal'));
  const thumbsUp = '\\u{1F44D}';
  term.write('xp', () => {
    term.write('\\r' + thumbsUp, () => {
      const line = term.buffer.active.getLine(0);
      term.select(0, 0, 2);
      window.__captainUnicodeResult = {
        activeVersion: term.unicode.activeVersion,
        text: line.translateToString(true),
        firstWidth: line.getCell(0).getWidth(),
        continuationWidth: line.getCell(1).getWidth(),
        selection: term.getSelection(),
      };
    });
  });
</script></body></html>`;

const assets = new Map([
  ["/", ["text/html; charset=utf-8", Buffer.from(testPage)]],
  ["/xterm.js", ["text/javascript; charset=utf-8", xtermJs]],
  ["/addon-unicode11.js", ["text/javascript; charset=utf-8", unicode11Js]],
  ["/xterm.css", ["text/css; charset=utf-8", xtermCss]],
]);

const server = createServer((request, response) => {
  const asset = assets.get(new URL(request.url || "/", "http://127.0.0.1").pathname);
  if (!asset) {
    response.writeHead(404).end();
    return;
  }
  response.writeHead(200, { "content-type": asset[0], "cache-control": "no-store" });
  response.end(asset[1]);
});

await new Promise((resolveListen, rejectListen) => {
  server.once("error", rejectListen);
  server.listen(0, "127.0.0.1", resolveListen);
});

let browser;
try {
  const playwright = await importPlaywright();
  browser = await playwright.chromium.launch({ headless: true });
  const page = await browser.newPage();
  const address = server.address();
  await page.goto(`http://127.0.0.1:${address.port}/`, { waitUntil: "load" });
  await page.waitForFunction(() => Boolean(window.__captainUnicodeResult));
  const result = await page.evaluate(() => window.__captainUnicodeResult);
  const thumbsUp = "\u{1F44D}";

  if (
    result.activeVersion !== "11"
    || result.text !== thumbsUp
    || result.firstWidth !== 2
    || result.continuationWidth !== 0
    || result.selection !== thumbsUp
  ) {
    throw new Error(`Unicode width regression: ${JSON.stringify(result)}`);
  }
  console.log("web terminal Unicode 11 width smoke: PASS");
} finally {
  if (browser) await browser.close();
  await new Promise((resolveClose) => server.close(resolveClose));
}
