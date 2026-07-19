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
import {
  providerDurationLabel,
  providerQuotaGroups,
  providerQuotaTone,
  providerQuotaWindows,
  providerResetLabel,
  providerSubscriptionFromBudget,
} from '../crates/captain-api/static/js/app/provider_quota_model.mjs';
import { withQuery } from '../crates/captain-api/static/js/app/api.js';

assert.deepEqual(
  PRIMARY_HUBS.map((hub) => hub.route),
  ['chat', 'projects', 'automation', 'learning', 'capabilities', 'status'],
);
assert.deepEqual(
  AUTOMATION_TABS.map((tab) => tab.route),
  ['workflows', 'triggers', 'crons', 'approvals', 'webhooks'],
);
assert.deepEqual(
  CAPABILITY_TABS.map((tab) => tab.route),
  ['native-capabilities', 'skills', 'tools'],
);
assert.equal(automationTabForRoute('automation').route, 'workflows');
assert.equal(capabilityTabForRoute('hands').route, 'native-capabilities');
assert.equal(capabilityTabForRoute('capabilities').route, 'native-capabilities');
assert.equal(hubForRoute('workflows'), 'automation');
assert.equal(hubForRoute('hands'), 'capabilities');
assert.equal(hubForRoute('system'), 'status');
assert.equal(
  withQuery('/api/capabilities/native', { scope: 'project', workspace: '/srv/My project', empty: '' }),
  '/api/capabilities/native?scope=project&workspace=%2Fsrv%2FMy+project',
);

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
  budget: {
    total_tokens_used: 12345,
    limited_agents: 4,
    operator_actions: [],
    provider_subscriptions: {
      state: 'warning',
      reported_by_provider: true,
      items: [{
        provider: 'codex',
        limit_id: 'codex',
        limit_name: 'Codex',
        plan_type: 'pro',
        alert_level: 'warning',
        stale: false,
        source: 'account_status',
        primary: { used_percent: 72.5, window_seconds: 18000, resets_at: '2026-07-18T18:00:00Z' },
        secondary: { used_percent: 41, window_seconds: 604800 },
      }],
    },
  },
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
assert.equal(snapshot.budget.provider.state, 'warning');
assert.equal(snapshot.budget.provider.reported, true);
assert.equal(snapshot.budget.provider.items[0].primary.windowSeconds, 18000);
assert.equal(snapshot.budget.provider.items[0].secondary.windowSeconds, 604800);
assert.equal(formatDuration(snapshot.uptimeSeconds), '1j 1h');
assert.equal(formatLatency(snapshot.streaming.firstTokenMs), '1.3s');
assert.equal(stateTone('critical'), 'err');

const providerStatus = providerSubscriptionFromBudget({
  provider_subscriptions: {
    state: 'warning',
    reported_by_provider: true,
    items: [{
      provider: 'codex',
      limit_id: 'codex',
      limit_name: 'Codex',
      primary: { used_percent: 72.5, window_seconds: 18000, reset_after_seconds: 3600 },
      secondary: { used_percent: 41, window_seconds: 604800 },
    }],
  },
});
const providerWindows = providerQuotaWindows(providerStatus);
assert.equal(providerWindows.length, 2);
assert.equal(providerDurationLabel(providerWindows[0].windowSeconds), '5h');
assert.equal(providerDurationLabel(providerWindows[1].windowSeconds), '1sem');
assert.equal(providerQuotaTone(providerWindows[0]), 'warn');
assert.equal(
  providerResetLabel(providerWindows[0], new Date(2026, 6, 18, 20, 0, 0)),
  '21:00',
);

const mixedProviderStatus = providerSubscriptionFromBudget({
  provider_subscriptions: {
    state: 'warning',
    reported_by_provider: true,
    items: [
      {
        provider: 'codex',
        limit_id: 'codex',
        limit_name: 'Codex',
        primary: { used_percent: 71, window_seconds: 604800 },
      },
      {
        provider: 'codex',
        limit_id: 'codex_bengalfox',
        limit_name: 'GPT-5.3-Codex-Spark',
        primary: { used_percent: 5, window_seconds: 604800 },
      },
    ],
  },
});
const activeSolGroups = providerQuotaGroups(mixedProviderStatus, 'codex/gpt-5.6-sol');
assert.equal(activeSolGroups.windows.length, 1);
assert.equal(activeSolGroups.windows[0].limitName, 'Codex');
assert.equal(activeSolGroups.alternativeLimitCount, 1);
assert.equal(activeSolGroups.alternativeTone, 'ok');
const activeSparkGroups = providerQuotaGroups(
  mixedProviderStatus,
  'codex/gpt-5.3-codex-spark',
);
assert.equal(activeSparkGroups.windows.length, 2);
assert.equal(activeSparkGroups.alternativeLimitCount, 0);

const criticalAlternativeStatus = providerSubscriptionFromBudget({
  provider_subscriptions: {
    state: 'critical',
    reported_by_provider: true,
    items: [
      mixedProviderStatus.items[0],
      {
        ...mixedProviderStatus.items[1],
        alert_level: 'critical',
        primary: { used_percent: 95, window_seconds: 604800 },
      },
    ],
  },
});
const criticalAlternativeGroups = providerQuotaGroups(
  criticalAlternativeStatus,
  'codex/gpt-5.6-sol',
);
assert.equal(criticalAlternativeGroups.alternativeLimitCount, 1);
assert.equal(criticalAlternativeGroups.alternativeTone, 'err');

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
const nativeCapabilitiesSource = await readFile(
  new URL('../crates/captain-api/static/js/app/views/NativeCapabilities.js', import.meta.url),
  'utf8',
);
const chatSource = await readFile(
  new URL('../crates/captain-api/static/js/app/views/Chat.js', import.meta.url),
  'utf8',
);
assert.match(apiSource, /budget:\s*\(\)\s*=>\s*request\('\/api\/budget'\)/);
assert.match(apiSource, /modelUpdates:\s*\(\)\s*=>\s*request\('\/api\/models\/updates'\)/);
assert.match(apiSource, /decideModelUpdate:.*\/api\/models\/updates\/decision/);
assert.match(apiSource, /nativeCapabilities:.*\/api\/capabilities\/native/);
assert.match(apiSource, /decideNativeCapability:/);
assert.match(apiSource, /rollbackNativeCapability:/);
assert.match(apiSource, /disableNativeCapability:/);
assert.match(shellSource, /Nouveau modèle Codex/);
assert.match(shellSource, /decision === 'switch'/);
assert.match(shellSource, /session_strategy/);
assert.match(shellSource, /Nouvelle session/);
assert.match(shellSource, /Résumé compact/);
assert.match(shellSource, /Conserver/);
assert.match(nativeCapabilitiesSource, /expected_hash:\s*item\.pending_hash/);
assert.match(nativeCapabilitiesSource, /include_source:\s*includeSource/);
assert.match(nativeCapabilitiesSource, /if \(!disableArmed\)/);
assert.match(chatSource, /ProviderQuotaBar/);
assert.match(chatSource, /PROVIDER_QUOTA_REFRESH_MS/);
assert.match(chatSource, /role="progressbar"/);

console.log('Captain Control contract passed: six hubs, native-first capabilities, live Status, and explicit operator decisions.');
