import test from 'node:test';
import assert from 'node:assert/strict';
import {
  providerQuotaGroups,
  providerQuotaWindows,
  providerSpendControls,
} from './provider_quota_model.mjs';

const status = {
  state: 'warning',
  reported: true,
  items: [{
    provider: 'codex',
    limit_id: 'codex',
    limit_name: 'Codex',
    alert_level: 'warning',
    primary: {
      used_percent: 63,
      remaining_percent: 37,
      remaining_source: 'derived_from_provider_used_percent',
      window_seconds: 18000,
    },
    spend_control: {
      reached: false,
      individual_limit: {
        source: 'monthly',
        limit: '200.00',
        used: '56.00',
        remaining: '144.00',
        used_percent: 28,
        remaining_percent: 72,
        remaining_source: 'provider_reported',
        reset_after_seconds: 86400,
      },
    },
  }],
};

test('rolling gauges use explicit remaining capacity while retaining pressure usage', () => {
  const [window] = providerQuotaWindows(status);
  assert.equal(window.usedPercent, 63);
  assert.equal(window.remainingPercent, 37);
  assert.equal(window.remainingSource, 'derived_from_provider_used_percent');
});

test('monthly spend control preserves provider-reported monetary headroom', () => {
  const [spend] = providerSpendControls(status);
  assert.equal(spend.remaining, '144.00');
  assert.equal(spend.limit, '200.00');
  assert.equal(spend.remainingPercent, 72);
  assert.equal(spend.remainingSource, 'provider_reported');
});

test('active model grouping carries rolling and monthly controls together', () => {
  const groups = providerQuotaGroups(status, 'codex/gpt-5.6-sol');
  assert.equal(groups.windows.length, 1);
  assert.equal(groups.spendControls.length, 1);
  assert.equal(groups.alternativeLimitCount, 0);
});
