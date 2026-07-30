// Exact browser-side batching for Captain chat deltas.

export const CONTROL_STREAM_FRAME_MS = 34;
export const CONTROL_STREAM_MAX_PENDING_CHARS = 32768;

export class TextDeltaBatcher {
  constructor(commit, options = {}) {
    if (typeof commit !== 'function') throw new TypeError('commit must be a function');
    this.commit = commit;
    this.schedule = options.schedule || ((callback, delay) => setTimeout(callback, delay));
    this.cancel = options.cancel || ((handle) => clearTimeout(handle));
    this.frameMs = options.frameMs || CONTROL_STREAM_FRAME_MS;
    this.maxPendingChars = options.maxPendingChars || CONTROL_STREAM_MAX_PENDING_CHARS;
    this.pending = '';
    this.timer = null;
    this.commits = 0;
  }

  push(content) {
    if (typeof content !== 'string' || content.length === 0) return;
    this.pending += content;
    if (this.pending.length >= this.maxPendingChars) {
      this.flush();
      return;
    }
    if (this.timer === null) {
      this.timer = this.schedule(() => {
        this.timer = null;
        this.flush();
      }, this.frameMs);
    }
  }

  flush() {
    if (this.timer !== null) {
      this.cancel(this.timer);
      this.timer = null;
    }
    if (!this.pending) return false;
    const content = this.pending;
    this.pending = '';
    this.commits += 1;
    this.commit(content);
    return true;
  }

  clear() {
    if (this.timer !== null) {
      this.cancel(this.timer);
      this.timer = null;
    }
    this.pending = '';
  }

  snapshot() {
    return {
      pendingChars: this.pending.length,
      commits: this.commits,
      scheduled: this.timer !== null,
    };
  }
}

export function textDeltaFromMessage(message) {
  if (message?.type === 'text_delta') return stringOrEmpty(message.content);
  if (message?.type === 'broadcast' && message.event?.TextDelta) {
    return stringOrEmpty(message.event.TextDelta.delta);
  }
  return null;
}

export function isScrollNearBottom(scrollHeight, scrollTop, clientHeight, threshold = 300) {
  return Math.max(0, Number(scrollHeight) - Number(scrollTop) - Number(clientHeight)) < threshold;
}

function stringOrEmpty(value) {
  return typeof value === 'string' ? value : '';
}
