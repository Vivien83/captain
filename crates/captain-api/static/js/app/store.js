// Captain Control — tiny global store (pub/sub over a plain object).
// Deliberately minimal: Preact re-renders subscribed components on publish.

const state = {
  authed: null,          // null = unknown, false = login needed, true = ok
  authMode: 'session',   // "session" | "apikey" | "none" | "unknown"
  agents: [],
  currentAgentId: null,
  sessions: [],
  currentSessionId: null,
  approvalsCount: 0,
  backgroundActivity: [], // [{key, label}]
  daemon: { ok: null, version: '' },
  toasts: [],            // [{id, kind, text}]
};

const listeners = new Set();

export function getState() { return state; }

export function setState(patch) {
  Object.assign(state, patch);
  listeners.forEach((fn) => fn(state));
}

export function subscribe(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

let toastSeq = 0;
export function toast(text, kind = 'ok', ttlMs = 4200) {
  const id = ++toastSeq;
  setState({ toasts: [...state.toasts, { id, kind, text }] });
  setTimeout(() => {
    setState({ toasts: state.toasts.filter((t) => t.id !== id) });
  }, ttlMs);
}
