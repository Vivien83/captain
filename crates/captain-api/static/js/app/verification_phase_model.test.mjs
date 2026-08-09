import assert from 'node:assert/strict';
import test from 'node:test';

import { deliveryVerificationFromPhase } from './verification_phase_model.mjs';

test('delivery verification is active only for verifying and correcting', () => {
  assert.deepEqual(deliveryVerificationFromPhase('verifying'), {
    phase: 'verifying',
    label: 'Vérification de la livraison',
  });
  assert.deepEqual(deliveryVerificationFromPhase('correcting'), {
    phase: 'correcting',
    label: 'Correction ciblée',
  });
});

test('terminal phases clear the ephemeral state without creating history', () => {
  const active = deliveryVerificationFromPhase('verifying');
  assert.equal(deliveryVerificationFromPhase('verification_verified', active), null);
  assert.equal(deliveryVerificationFromPhase('verification_incomplete', active), null);
  assert.equal(deliveryVerificationFromPhase('done', active), null);
  assert.equal(deliveryVerificationFromPhase('error', active), null);
});

test('unrelated phases preserve the current verification state', () => {
  const active = deliveryVerificationFromPhase('correcting');
  assert.equal(deliveryVerificationFromPhase('thinking', active), active);
  assert.equal(deliveryVerificationFromPhase('tool_use', active), active);
});
