// Provider-owned subscription quota model shared by the web and desktop UI.

export const PROVIDER_QUOTA_REFRESH_MS = 5000;

export function providerSubscriptionFromBudget(payload) {
  const source = payload && typeof payload.provider_subscriptions === 'object'
    ? payload.provider_subscriptions
    : {};
  return {
    state: typeof source.state === 'string' ? source.state : 'unavailable',
    reported: source.reported_by_provider === true,
    items: Array.isArray(source.items) ? source.items : [],
  };
}

export function providerQuotaWindows(status) {
  const windows = [];
  for (const item of status.items || []) {
    for (const [kind, value] of [['primary', item.primary], ['secondary', item.secondary]]) {
      if (!value || !Number.isFinite(Number(value.used_percent))) continue;
      windows.push({
        provider: item.provider || 'provider',
        limitId: item.limit_id || 'quota',
        limitName: item.limit_name || item.limit_id || 'Quota',
        planType: item.plan_type || null,
        alertLevel: item.alert_level || 'normal',
        stale: item.stale === true,
        blocked: Boolean(item.rate_limit_reached_type) || item.alert_level === 'exhausted',
        kind,
        usedPercent: Math.max(0, Math.min(100, Number(value.used_percent))),
        windowSeconds: numberOrNull(value.window_seconds),
        resetAfterSeconds: numberOrNull(value.reset_after_seconds),
        resetsAt: typeof value.resets_at === 'string' ? value.resets_at : null,
      });
    }
  }
  return windows;
}

export function providerQuotaGroups(status, activeModel = '') {
  const identity = modelIdentity(activeModel);
  const providerItems = (status.items || []).filter((item) => (
    !identity.provider || providerIdsMatch(item.provider || '', identity.provider)
  ));
  const applicableItems = providerItems.filter((item) => (
    providerQuotaAppliesToModel(item, identity.model)
  ));
  const alternativeItems = providerItems.filter((item) => (
    !providerQuotaAppliesToModel(item, identity.model)
  ));
  const alternativeTone = alternativeItems.some(providerQuotaItemIsCritical)
    ? 'err'
    : alternativeItems.some(providerQuotaItemIsWarning) ? 'warn' : 'ok';
  return {
    providerItems,
    windows: providerQuotaWindows({ ...status, items: applicableItems }),
    alternativeLimitCount: alternativeItems.length,
    alternativeTone,
    hasProviderObservation: status.reported === true && providerItems.length > 0,
  };
}

export function providerQuotaMeta(status, activeModel = '') {
  const groups = providerQuotaGroups(status, activeModel);
  const first = groups.providerItems[0] || {};
  const credits = groups.providerItems.map((item) => item.credits).find(Boolean) || null;
  let creditsLabel = null;
  if (credits) {
    if (credits.unlimited === true) creditsLabel = 'crédits ∞';
    else if (credits.balance !== undefined && credits.balance !== null) creditsLabel = `crédits ${credits.balance}`;
    else if (credits.has_credits === true) creditsLabel = 'crédits disponibles';
    else creditsLabel = 'crédits épuisés';
  }
  return {
    provider: codexDisplayName(first.provider || 'Provider'),
    activeModel: modelIdentity(activeModel).model || null,
    planType: first.plan_type || null,
    creditsLabel,
  };
}

export function providerDurationLabel(seconds, fallback = 'fenêtre') {
  if (!Number.isFinite(seconds) || seconds < 0) return fallback;
  if (seconds !== 0 && seconds % 604800 === 0) return `${seconds / 604800}sem`;
  if (seconds !== 0 && seconds % 86400 === 0) return `${seconds / 86400}j`;
  if (seconds !== 0 && seconds % 3600 === 0) return `${seconds / 3600}h`;
  if (seconds !== 0 && seconds % 60 === 0) return `${seconds / 60}m`;
  return `${seconds}s`;
}

export function providerResetLabel(window, now = new Date()) {
  let reset = window.resetsAt ? new Date(window.resetsAt) : null;
  if (!reset || Number.isNaN(reset.getTime())) {
    reset = Number.isFinite(window.resetAfterSeconds)
      ? new Date(now.getTime() + window.resetAfterSeconds * 1000)
      : null;
  }
  if (!reset) return 'reprise inconnue';

  const dayDelta = localDayNumber(reset) - localDayNumber(now);
  const time = `${two(reset.getHours())}:${two(reset.getMinutes())}`;
  if (dayDelta < 0 || dayDelta > 6) return `${two(reset.getDate())}/${two(reset.getMonth() + 1)} ${time}`;
  if (dayDelta === 0) return time;
  if (dayDelta === 1) return `demain ${time}`;
  return `${['dim', 'lun', 'mar', 'mer', 'jeu', 'ven', 'sam'][reset.getDay()]} ${time}`;
}

export function providerQuotaTone(window) {
  if (window.blocked || window.usedPercent >= 90) return 'err';
  if (window.stale || window.usedPercent >= 70) return 'warn';
  return 'ok';
}

function numberOrNull(value) {
  if (value === null || value === undefined || value === '') return null;
  const number = Number(value);
  return Number.isFinite(number) && number >= 0 ? number : null;
}

function codexDisplayName(value) {
  return value === 'codex' || value === 'openai-codex' ? 'Codex' : value;
}

function providerQuotaAppliesToModel(item, activeModel) {
  if (providerQuotaIsGeneral(item)) return true;
  const model = normalizeIdentifier(activeModel);
  return Boolean(model) && [item.limit_id, item.limit_name]
    .some((candidate) => normalizeIdentifier(candidate) === model);
}

function providerQuotaIsGeneral(item) {
  const limitId = canonicalProviderId(item.limit_id);
  const provider = canonicalProviderId(item.provider);
  const limitName = normalizeIdentifier(item.limit_name);
  const providerName = normalizeIdentifier(codexDisplayName(item.provider || ''));
  return Boolean(limitId && provider && limitId === provider)
    || Boolean(limitName && providerName && limitName === providerName);
}

function providerQuotaItemIsCritical(item) {
  return Boolean(item.rate_limit_reached_type)
    || ['critical', 'exhausted'].includes(item.alert_level)
    || providerQuotaItemMaxPercent(item) >= 90;
}

function providerQuotaItemIsWarning(item) {
  return providerQuotaItemIsCritical(item)
    || item.alert_level === 'warning'
    || providerQuotaItemMaxPercent(item) >= 70;
}

function providerQuotaItemMaxPercent(item) {
  return Math.max(0, ...[item.primary, item.secondary]
    .map((window) => Number(window && window.used_percent))
    .filter(Number.isFinite));
}

function providerIdsMatch(left, right) {
  return canonicalProviderId(left) === canonicalProviderId(right);
}

function canonicalProviderId(value) {
  const normalized = String(value || '').trim().toLowerCase();
  return normalized === 'openai-codex' ? 'codex' : normalized;
}

function modelIdentity(value) {
  const [provider = '', ...model] = String(value || '').split('/');
  return model.length > 0
    ? { provider: provider.trim(), model: model.join('/').trim() }
    : { provider: provider.trim(), model: '' };
}

function normalizeIdentifier(value) {
  return String(value || '').toLowerCase().replace(/[^a-z0-9]/g, '');
}

function localDayNumber(value) {
  return Date.UTC(value.getFullYear(), value.getMonth(), value.getDate()) / 86400000;
}

function two(value) {
  return String(value).padStart(2, '0');
}
