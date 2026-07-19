#!/usr/bin/env node

import { mkdir, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const base = process.env.CAPSPEC_CERT_BASE || "http://127.0.0.1:50481";
const artifactRoot = resolve(process.env.CAPSPEC_CERT_CONTROL_ARTIFACTS || "target/capspec-control-certification");
const username = process.env.CAPSPEC_CERT_CONTROL_USERNAME || "certifier";
const password = process.env.CAPSPEC_CERT_CONTROL_PASSWORD || "capspec-control-password";

async function importPlaywright() {
  const candidates = [];
  if (process.env.CAPTAIN_PLAYWRIGHT_MODULE) candidates.push(process.env.CAPTAIN_PLAYWRIGHT_MODULE);
  candidates.push("playwright");
  try {
    const globalRoot = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
    candidates.push(join(globalRoot, "playwright/index.mjs"));
    candidates.push(join(globalRoot, "playwright/index.js"));
  } catch {
    // A repository-local Playwright installation may still satisfy the import.
  }
  const failures = [];
  for (const candidate of candidates) {
    try {
      return await import(candidate.startsWith("/") ? pathToFileURL(candidate).href : candidate);
    } catch (error) {
      failures.push(`${candidate}: ${error.message}`);
    }
  }
  throw new Error(`Playwright is required:\n${failures.join("\n")}`);
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function auditViewport(browser, name, viewport) {
  const context = await browser.newContext({ viewport, colorScheme: "dark", reducedMotion: "reduce" });
  const page = await context.newPage();
  const browserErrors = [];
  page.on("pageerror", (error) => browserErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push(message.text());
  });
  page.on("requestfailed", (request) => {
    browserErrors.push(`${request.method()} ${request.url()}: ${request.failure()?.errorText}`);
  });

  const response = await page.goto(`${base}/#/native-capabilities`, { waitUntil: "networkidle" });
  assert(response?.status() === 200, `${name}: Control returned ${response?.status()}`);
  await page.locator('input[autocomplete="username"]').fill(username);
  await page.locator('input[autocomplete="current-password"]').fill(password);
  await page.locator('button[type="submit"]').click();
  await page.locator(".native-capabilities").waitFor();
  await page.locator(".native-row-toggle", { hasText: "cert-parallel" }).waitFor();
  assert((await page.locator("h1").textContent())?.trim() === "Capabilities", `${name}: wrong hub title`);
  assert(await page.locator(".hub-tab.active", { hasText: "Natives" }).count() === 1, `${name}: Natives is not active`);
  assert(await page.locator(".native-run-entry", { hasText: "cert-parallel" }).count() >= 1, `${name}: durable run missing`);

  await page.locator(".native-row-toggle", { hasText: "cert-parallel" }).click();
  await page.locator(".native-capability-detail").waitFor();
  assert(await page.locator(".native-capability-detail").filter({ hasText: /visions/ }).count() === 1, `${name}: revisions missing`);

  const overflow = await page.evaluate(() => ({
    body: document.body.scrollWidth - document.body.clientWidth,
    native: (() => {
      const element = document.querySelector(".native-capabilities");
      return element ? element.scrollWidth - element.clientWidth : -1;
    })(),
  }));
  assert(overflow.body <= 1 && overflow.native <= 1, `${name}: horizontal overflow ${JSON.stringify(overflow)}`);
  assert(browserErrors.length === 0, `${name}: browser errors: ${browserErrors.join(" | ")}`);

  await page.screenshot({ path: join(artifactRoot, `control-${name}.png`), fullPage: true });
  await context.close();
  return { name, viewport, overflow, browser_errors: browserErrors };
}

await mkdir(artifactRoot, { recursive: true });
const { chromium } = await importPlaywright();
const browser = await chromium.launch({ headless: true });
try {
  const results = [];
  results.push(await auditViewport(browser, "desktop", { width: 1440, height: 900 }));
  results.push(await auditViewport(browser, "fold6", { width: 344, height: 882 }));
  const summary = { status: "passed", base, results };
  await writeFile(join(artifactRoot, "control-summary.json"), `${JSON.stringify(summary, null, 2)}\n`, "utf8");
  process.stdout.write(`${JSON.stringify(summary)}\n`);
} finally {
  await browser.close();
}
