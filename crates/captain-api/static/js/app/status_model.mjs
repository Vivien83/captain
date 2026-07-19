const objectAt = (value) => (value && typeof value === 'object' && !Array.isArray(value) ? value : {});
const arrayAt = (value) => (Array.isArray(value) ? value : []);
const numberAt = (value) => (Number.isFinite(Number(value)) ? Number(value) : 0);
const optionalNumber = (value) => (value === null || value === undefined || value === '' ? null : numberAt(value));
const optionalBoolean = (value) => (typeof value === 'boolean' ? value : null);
const stringAt = (value, fallback = '') => (typeof value === 'string' && value ? value : fallback);
const stringsAt = (value) => arrayAt(value).filter((item) => typeof item === 'string');

export function statusSnapshot(payload = {}) {
  const body = objectAt(payload);
  const health = objectAt(body.runtime_health);
  const channels = objectAt(body.channels);
  const inbound = objectAt(channels.inbound_queue);
  const toolRuns = objectAt(body.tool_runs);
  const agentApi = objectAt(body.agent_api);
  const egress = objectAt(agentApi.egress_queue);
  const consciousness = objectAt(body.consciousness);
  const streaming = objectAt(body.streaming);
  const streamingLast = objectAt(streaming.last);
  const shutdown = objectAt(body.shutdown);
  const disk = objectAt(body.disk);
  const embeddings = objectAt(body.native_embeddings);
  const voice = objectAt(body.native_voice);
  const budget = objectAt(body.budget);
  const providerSubscriptions = objectAt(budget.provider_subscriptions);
  const providerQuotaItems = arrayAt(providerSubscriptions.items).map((item) => {
    const quota = objectAt(item);
    const window = (value) => {
      const entry = objectAt(value);
      return Object.keys(entry).length === 0 ? null : {
        usedPercent: numberAt(entry.used_percent),
        windowSeconds: optionalNumber(entry.window_seconds),
        resetsAt: stringAt(entry.resets_at),
      };
    };
    return {
      provider: stringAt(quota.provider, 'provider'),
      id: stringAt(quota.limit_id, 'quota'),
      name: stringAt(quota.limit_name, stringAt(quota.limit_id, 'quota')),
      plan: stringAt(quota.plan_type),
      alert: stringAt(quota.alert_level, 'normal'),
      stale: quota.stale === true,
      source: stringAt(quota.source, 'unknown'),
      primary: window(quota.primary),
      secondary: window(quota.secondary),
    };
  });
  const workload = objectAt(body.workload);
  const projects = objectAt(workload.projects);
  const goals = objectAt(workload.goals);
  const automation = objectAt(workload.automation);
  const delivery = objectAt(automation.delivery);

  return {
    state: stringAt(body.status, 'unknown'),
    version: stringAt(body.version, '?'),
    uptimeSeconds: numberAt(body.uptime_seconds ?? body.uptime_secs),
    provider: stringAt(body.default_provider ?? body.provider, '—'),
    model: stringAt(body.default_model ?? body.model, '—'),
    llmReady: body.llm_driver_ready === true,
    health: {
      state: stringAt(health.state, body.llm_driver_ready === false ? 'critical' : 'unknown'),
      issueCount: numberAt(health.issue_count),
      issues: arrayAt(health.issues).map((issue) => ({
        kind: stringAt(objectAt(issue).kind, 'issue'),
        severity: stringAt(objectAt(issue).severity, 'watch'),
        summary: stringAt(objectAt(issue).summary),
        action: stringAt(objectAt(issue).action),
      })),
      actions: stringsAt(health.operator_actions),
    },
    agents: numberAt(body.agent_count),
    activeRuns: numberAt(body.active_run_count),
    processes: numberAt(body.process_count),
    channels: {
      ready: numberAt(channels.ready_count),
      total: numberAt(channels.total ?? body.channel_total),
      configured: numberAt(channels.configured_count ?? body.channel_configured_count),
      locked: arrayAt(channels.locked).length,
      readyNames: stringsAt(channels.ready),
      pendingMessages: numberAt(inbound.pending_messages),
      deadLetters: numberAt(inbound.dead_letter_messages),
    },
    toolRuns: {
      running: numberAt(toolRuns.running),
      completed: numberAt(toolRuns.completed),
      failed: numberAt(toolRuns.failed),
      interrupted: numberAt(toolRuns.interrupted),
    },
    agentApi: {
      state: stringAt(egress.state, 'unknown'),
      pending: numberAt(egress.pending),
      due: numberAt(egress.due),
      deadLetters: numberAt(egress.dead_letters),
    },
    consciousness: {
      state: stringAt(consciousness.state, 'unknown'),
      signals: stringsAt(consciousness.signals),
      actions: stringsAt(consciousness.operator_actions),
    },
    streaming: {
      active: numberAt(streaming.active),
      completed: numberAt(streaming.completed),
      firstSignalMs: optionalNumber(streamingLast.first_signal_ms),
      firstTokenMs: optionalNumber(streamingLast.first_token_ms),
      totalMs: optionalNumber(streamingLast.total_ms),
    },
    shutdown: {
      status: stringAt(shutdown.status, 'unknown'),
      activeWork: numberAt(shutdown.active_work_count ?? shutdown.active_run_count),
    },
    disk: {
      availableGiB: optionalNumber(disk.available_gib),
      cleanupRecommended: disk.cleanup_recommended === true,
    },
    native: {
      embeddings: optionalBoolean(embeddings.ready),
      tts: optionalBoolean(voice.tts_ready),
      stt: optionalBoolean(voice.stt_ready),
    },
    budget: {
      totalTokens: numberAt(budget.total_tokens_used),
      limitedAgents: numberAt(budget.limited_agents),
      actions: stringsAt(budget.operator_actions),
      provider: {
        state: stringAt(providerSubscriptions.state, 'unavailable'),
        reported: providerSubscriptions.reported_by_provider === true,
        items: providerQuotaItems,
      },
    },
    workload: {
      projectAttention: numberAt(projects.attention_count),
      projectsActive: numberAt(projects.active),
      goalsActive: numberAt(goals.active),
      goalsEscalated: numberAt(goals.escalated),
      cronEnabled: numberAt(automation.cron_enabled),
      cronDue: numberAt(automation.cron_due),
      deliveryDue: numberAt(delivery.redelivery_due),
      deliveryDeadLetters: numberAt(delivery.dead_letters),
    },
    access: {
      authMode: stringAt(body.auth_mode, 'unknown'),
      networkEnabled: body.network_enabled === true,
    },
    raw: body,
  };
}

export function stateTone(state) {
  const normalized = String(state || '').toLowerCase();
  if (['ok', 'ready', 'running', 'steady', 'idle', 'healthy'].includes(normalized)) return 'ok';
  if (['watch', 'warn', 'warning', 'attention', 'draining', 'degraded'].includes(normalized)) return 'warn';
  if (['critical', 'error', 'failed', 'unavailable', 'offline'].includes(normalized)) return 'err';
  return 'neutral';
}

export function formatDuration(seconds) {
  const value = numberAt(seconds);
  const days = Math.floor(value / 86400);
  const hours = Math.floor((value % 86400) / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  if (days > 0) return `${days}j ${hours}h`;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, '0')}`;
  return `${minutes}min`;
}

export function formatLatency(milliseconds) {
  if (milliseconds === null || milliseconds === undefined) return '—';
  const value = numberAt(milliseconds);
  if (value >= 1000) return `${(value / 1000).toFixed(value >= 10000 ? 0 : 1)}s`;
  return `${Math.round(value)}ms`;
}
