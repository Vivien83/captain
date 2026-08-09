const ACTIVE_PHASES = Object.freeze({
  verifying: Object.freeze({ phase: 'verifying', label: 'Vérification de la livraison' }),
  correcting: Object.freeze({ phase: 'correcting', label: 'Correction ciblée' }),
});

const TERMINAL_PHASES = new Set([
  'verification_verified',
  'verification_incomplete',
  'done',
  'error',
]);

export function deliveryVerificationFromPhase(phase, current = null) {
  if (ACTIVE_PHASES[phase]) return ACTIVE_PHASES[phase];
  if (TERMINAL_PHASES.has(phase)) return null;
  return current;
}
