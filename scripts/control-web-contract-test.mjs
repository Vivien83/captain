import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  AUTOMATION_TABS,
  CAPABILITY_TABS,
  PRIMARY_HUBS,
  automationTabForRoute,
  capabilityTabForRoute,
  hubForRoute,
} from '../crates/captain-api/static/js/app/control_contract.mjs';
import {
  formatDuration,
  formatLatency,
  stateTone,
  statusSnapshot,
} from '../crates/captain-api/static/js/app/status_model.mjs';

assert.deepEqual(
  PRIMARY_HUBS.map((hub) => hub.route),
  ['chat', 'projects', 'automation', 'learning', 'capabilities', 'status'],
);
assert.deepEqual(
  AUTOMATION_TABS.map((tab) => tab.route),
  ['workflows', 'triggers', 'crons', 'approvals', 'webhooks'],
);
assert.deepEqual(CAPABILITY_TABS.map((tab) => tab.route), ['skills', 'tools']);
assert.equal(automationTabForRoute('automation').route, 'workflows');
assert.equal(capabilityTabForRoute('hands').route, 'skills');
assert.equal(hubForRoute('workflows'), 'automation');
assert.equal(hubForRoute('hands'), 'capabilities');
assert.equal(hubForRoute('system'), 'status');

const snapshot = statusSnapshot({
  status: 'running',
  version: '0.1.0-dev',
  uptime_seconds: 90061,
  default_provider: 'codex',
  default_model: 'gpt-5.5',
  llm_driver_ready: true,
  agent_count: 4,
  active_run_count: 2,
  process_count: 1,
  runtime_health: {
    state: 'warn',
    issue_count: 1,
    issues: [{ kind: 'disk', severity: 'warn', summary: 'Disk low.', action: 'Clean debug.' }],
    operator_actions: ['Clean debug.'],
  },
  channels: {
    ready_count: 2,
    total: 4,
    configured_count: 3,
    ready: ['telegram', 'discord'],
    locked: ['email'],
    inbound_queue: { pending_messages: 1, dead_letter_messages: 0 },
  },
  tool_runs: { running: 3, completed: 40, failed: 1, interrupted: 2 },
  agent_api: { egress_queue: { state: 'attention', pending: 2, due: 1, dead_letters: 0 } },
  consciousness: { state: 'warn', signals: ['goal stalled'], operator_actions: ['Review goal.'] },
  streaming: { active: 1, completed: 8, last: { first_signal_ms: 450, first_token_ms: 1250, total_ms: 5100 } },
  shutdown: { status: 'idle', active_work_count: 0 },
  disk: { available_gib: 12.5, cleanup_recommended: true },
  native_embeddings: { ready: true },
  native_voice: { stt_ready: true, tts_ready: false },
  budget: { total_tokens_used: 12345, limited_agents: 4, operator_actions: [] },
  workload: {
    projects: { active: 2, attention_count: 1 },
    goals: { active: 1, escalated: 0 },
    automation: { cron_enabled: 5, cron_due: 1, delivery: { redelivery_due: 0, dead_letters: 0 } },
  },
  auth_mode: 'session',
  network_enabled: true,
});

assert.equal(snapshot.health.state, 'warn');
assert.equal(snapshot.health.issues[0].action, 'Clean debug.');
assert.equal(snapshot.toolRuns.interrupted, 2);
assert.equal(snapshot.agentApi.due, 1);
assert.equal(snapshot.streaming.firstTokenMs, 1250);
assert.equal(snapshot.workload.projectAttention, 1);
assert.equal(snapshot.native.tts, false);
assert.equal(formatDuration(snapshot.uptimeSeconds), '1j 1h');
assert.equal(formatLatency(snapshot.streaming.firstTokenMs), '1.3s');
assert.equal(stateTone('critical'), 'err');

const legacy = statusSnapshot({ agent_count: 2, provider: 'codex', model: 'gpt-5' });
assert.equal(legacy.agents, 2);
assert.equal(legacy.provider, 'codex');
assert.equal(legacy.health.state, 'unknown');

const apiSource = await readFile(
  new URL('../crates/captain-api/static/js/app/api.js', import.meta.url),
  'utf8',
);
const shellSource = await readFile(
  new URL('../crates/captain-api/static/js/app/components/Shell.js', import.meta.url),
  'utf8',
);
assert.match(apiSource, /modelUpdates:\s*\(\)\s*=>\s*request\('\/api\/models\/updates'\)/);
assert.match(apiSource, /decideModelUpdate:.*\/api\/models\/updates\/decision/);
assert.match(shellSource, /Nouveau modèle Codex/);
assert.match(shellSource, /decision === 'switch'/);
assert.match(shellSource, /session_strategy/);
assert.match(shellSource, /Nouvelle session/);
assert.match(shellSource, /Résumé compact/);
assert.match(shellSource, /Conserver/);

console.log('Captain Control contract passed: six hubs, workflow-first automation, live Status, and explicit Codex model decisions.');
