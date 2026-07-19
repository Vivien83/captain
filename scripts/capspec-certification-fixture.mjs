#!/usr/bin/env node

import { appendFile, mkdir, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { resolve } from "node:path";

const port = Number.parseInt(process.env.CAPSPEC_CERT_FIXTURE_PORT || "50482", 10);
const artifactRoot = resolve(process.env.CAPSPEC_CERT_ARTIFACTS || "target/capspec-certification-fixture");
const repoRoot = resolve(process.env.CAPSPEC_CERT_REPO_ROOT || process.cwd());
const workRoot = resolve(process.env.CAPSPEC_CERT_WORK_ROOT || artifactRoot);
const projectRoot = resolve(process.env.CAPSPEC_CERT_PROJECT_ROOT || workRoot);
const statePath = resolve(artifactRoot, "fixture-state.json");
const requestLogPath = resolve(artifactRoot, "fixture-requests.jsonl");

const state = {
  nextUpdateId: 1,
  nextMessageId: 1000,
  openaiRequests: [],
  telegramUpdates: [],
  telegramCalls: [],
};

const scenarios = {
  parallel: { tool: "cap_cert_parallel", input: { root: repoRoot } },
  transform: { tool: "cap_cert_transform", input: {} },
  write: { tool: "cap_cert_write", input: { root: workRoot } },
  cargo: { tool: "cap_cert_cargo", input: {} },
  "http-allowed": { tool: "cap_cert_http_allowed", input: {} },
  "http-denied": { tool: "cap_cert_http_denied", input: {} },
  memory: { tool: "cap_cert_memory", input: {} },
  traversal: { tool: "cap_cert_traversal", input: { root: workRoot } },
  secret: { tool: "cap_cert_secret", input: {} },
  crash: { tool: "cap_cert_crash", input: { root: workRoot } },
  scope: { tool: "cap_cert_scope", input: { root: projectRoot } },
  api: { tool: "cap_cert_parallel", input: { root: repoRoot } },
  tui: { tool: "cap_cert_parallel", input: { root: repoRoot } },
  telegram: { tool: "cap_cert_parallel", input: { root: repoRoot } },
};

await mkdir(artifactRoot, { recursive: true });
await persistState();

function json(response, status, body) {
  const payload = JSON.stringify(body);
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
    "cache-control": "no-store",
  });
  response.end(payload);
}

async function readBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw) return {};
  const type = request.headers["content-type"] || "";
  if (type.includes("application/json")) {
    try {
      return JSON.parse(raw);
    } catch {
      return { _raw: raw };
    }
  }
  if (type.includes("application/x-www-form-urlencoded")) {
    return Object.fromEntries(new URLSearchParams(raw));
  }
  return { _raw: raw.slice(0, 4096) };
}

function messageText(message) {
  if (typeof message?.content === "string") return message.content;
  if (!Array.isArray(message?.content)) return "";
  return message.content
    .map((part) => (typeof part === "string" ? part : part?.text || ""))
    .join("\n");
}

function certificationTurn(messages) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role !== "user") continue;
    const text = messageText(messages[index]);
    const match = text.match(/\[CAPSPEC-CERT:([a-z-]+)\]/i);
    if (!match) continue;
    const scenario = match[1].toLowerCase();
    const suffix = text.slice((match.index || 0) + match[0].length).trim();
    let inputOverride = {};
    const objectStart = suffix.indexOf("{");
    if (objectStart >= 0) {
      try {
        const candidate = JSON.parse(suffix.slice(objectStart));
        if (candidate && typeof candidate === "object" && !Array.isArray(candidate)) inputOverride = candidate;
      } catch {
        // Ordinary prose after the marker is intentionally ignored.
      }
    }
    const toolResults = messages.slice(index + 1).filter((message) => message?.role === "tool");
    return { scenario, toolResults, inputOverride };
  }
  return null;
}

function toolCall(name, input, requestNumber) {
  return {
    id: `cert_call_${requestNumber}_${name}`,
    type: "function",
    function: { name, arguments: JSON.stringify(input) },
  };
}

function nextCompletion(payload, requestNumber) {
  const turn = certificationTurn(payload.messages || []);
  if (!turn || !scenarios[turn.scenario]) {
    return { kind: "text", text: "CAPSPEC_CERT_FIXTURE_READY" };
  }
  if (turn.toolResults.length === 0) {
    const scenario = scenarios[turn.scenario];
    return {
      kind: "tool",
      call: toolCall(
        "capability_search",
        { query: scenario.tool.replace(/^cap_/, "").replaceAll("_", "-") },
        requestNumber,
      ),
    };
  }
  if (turn.toolResults.length === 1) {
    const scenario = scenarios[turn.scenario];
    return {
      kind: "tool",
      call: toolCall(scenario.tool, { ...scenario.input, ...turn.inputOverride }, requestNumber),
    };
  }
  return { kind: "text", text: `CAPSPEC_CERT_OK ${turn.scenario}` };
}

function completionBody(completion) {
  const message = completion.kind === "tool"
    ? { role: "assistant", content: null, tool_calls: [completion.call] }
    : { role: "assistant", content: completion.text, tool_calls: null };
  return {
    id: `chatcmpl-cert-${Date.now()}`,
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model: "captain-capspec-certifier",
    choices: [{ index: 0, message, finish_reason: completion.kind === "tool" ? "tool_calls" : "stop" }],
    usage: { prompt_tokens: 32, completion_tokens: 8, total_tokens: 40 },
  };
}

function streamCompletion(response, completion) {
  response.writeHead(200, {
    "content-type": "text/event-stream; charset=utf-8",
    "cache-control": "no-cache",
    connection: "keep-alive",
  });
  const delta = completion.kind === "tool"
    ? { role: "assistant", tool_calls: [{ index: 0, ...completion.call }] }
    : { role: "assistant", content: completion.text };
  response.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason: null }] })}\n\n`);
  response.write(`data: ${JSON.stringify({
    choices: [{ index: 0, delta: {}, finish_reason: completion.kind === "tool" ? "tool_calls" : "stop" }],
    usage: { prompt_tokens: 32, completion_tokens: 8, total_tokens: 40 },
  })}\n\n`);
  response.end("data: [DONE]\n\n");
}

async function recordOpenAi(payload, completion) {
  const row = {
    at: new Date().toISOString(),
    stream: payload.stream === true,
    model: payload.model,
    offered_tools: Array.isArray(payload.tools)
      ? payload.tools.map((tool) => tool?.function?.name).filter(Boolean)
      : [],
    decision: completion.kind === "tool" ? completion.call.function.name : completion.text,
  };
  state.openaiRequests.push(row);
  await record("openai", row);
}

async function record(kind, detail) {
  await appendFile(requestLogPath, `${JSON.stringify({ kind, ...detail })}\n`, "utf8");
  await persistState();
}

async function persistState() {
  await writeFile(statePath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
}

function telegramResult(method, body) {
  const chatId = Number.parseInt(String(body.chat_id || 4242), 10);
  const messageId = Number.parseInt(String(body.message_id || state.nextMessageId++), 10);
  if (method === "getMe") {
    return { id: 424242, is_bot: true, first_name: "Captain Cert", username: "captain_cert_bot" };
  }
  if (method === "getUpdates") {
    const updates = state.telegramUpdates.splice(0, state.telegramUpdates.length);
    return updates;
  }
  if (method === "sendMessage" || method === "sendMessageDraft" || method === "editMessageText") {
    return {
      message_id: messageId,
      date: Math.floor(Date.now() / 1000),
      chat: { id: chatId, type: "private" },
      text: body.text || "",
    };
  }
  return true;
}

async function handleTelegram(request, response, url) {
  const match = url.pathname.match(/^\/telegram\/bot[^/]+\/([A-Za-z0-9_]+)$/);
  if (!match) return false;
  const method = match[1];
  const body = await readBody(request);
  const row = { at: new Date().toISOString(), method, body };
  state.telegramCalls.push(row);
  await record("telegram", row);
  json(response, 200, { ok: true, result: telegramResult(method, body) });
  return true;
}

async function handleControl(request, response, url) {
  if (url.pathname === "/cert/health") {
    json(response, 200, { status: "ok" });
    return true;
  }
  if (url.pathname === "/cert/state" && request.method === "GET") {
    json(response, 200, state);
    return true;
  }
  if (url.pathname === "/cert/telegram/push" && request.method === "POST") {
    const update = await readBody(request);
    if (!Number.isInteger(update.update_id)) update.update_id = state.nextUpdateId++;
    state.telegramUpdates.push(update);
    await record("telegram_push", { at: new Date().toISOString(), update_id: update.update_id });
    json(response, 202, { status: "queued", update_id: update.update_id });
    return true;
  }
  return false;
}

const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url || "/", `http://127.0.0.1:${port}`);
    if (await handleControl(request, response, url)) return;
    if (await handleTelegram(request, response, url)) return;
    if (url.pathname === "/v1/chat/completions" && request.method === "POST") {
      const payload = await readBody(request);
      const requestNumber = state.openaiRequests.length + 1;
      const completion = nextCompletion(payload, requestNumber);
      await recordOpenAi(payload, completion);
      if (payload.stream === true) streamCompletion(response, completion);
      else json(response, 200, completionBody(completion));
      return;
    }
    json(response, 404, { error: "not found" });
  } catch (error) {
    json(response, 500, { error: String(error?.stack || error) });
  }
});

server.listen(port, "127.0.0.1", async () => {
  await writeFile(resolve(artifactRoot, "fixture-ready"), `${port}\n`, "utf8");
  process.stdout.write(`CapSpec certification fixture listening on 127.0.0.1:${port}\n`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
