#!/usr/bin/env node

import assert from 'node:assert/strict';
import {
  CONTROL_STREAM_FRAME_MS,
  TextDeltaBatcher,
  isScrollNearBottom,
  textDeltaFromMessage,
} from '../crates/captain-api/static/js/app/chat_stream_batcher.mjs';

const callbacks = new Map();
let nextHandle = 0;
const commits = [];
const batcher = new TextDeltaBatcher((content) => commits.push(content), {
  schedule(callback) {
    const handle = ++nextHandle;
    callbacks.set(handle, callback);
    return handle;
  },
  cancel(handle) { callbacks.delete(handle); },
});

for (let index = 0; index < 1000; index += 1) batcher.push(String(index % 10));
assert.equal(CONTROL_STREAM_FRAME_MS, 34);
assert.equal(callbacks.size, 1, 'one visual commit is scheduled for one burst');
assert.equal(batcher.snapshot().pendingChars, 1000);
callbacks.values().next().value();
assert.deepEqual(commits, ['0123456789'.repeat(100)]);
assert.deepEqual(batcher.snapshot(), { pendingChars: 0, commits: 1, scheduled: false });

batcher.push('before-tool');
assert.equal(batcher.flush(), true);
batcher.push('after-tool');
assert.equal(batcher.flush(), true);
assert.deepEqual(commits.slice(1), ['before-tool', 'after-tool']);

const bounded = [];
const capBatcher = new TextDeltaBatcher((content) => bounded.push(content), {
  schedule: () => 1,
  cancel: () => {},
  maxPendingChars: 8,
});
capBatcher.push('1234');
capBatcher.push('5678');
assert.deepEqual(bounded, ['12345678'], 'memory cap flushes without dropping text');

const clearCallbacks = new Map();
const cleared = [];
const clearBatcher = new TextDeltaBatcher((content) => cleared.push(content), {
  schedule(callback) { clearCallbacks.set(1, callback); return 1; },
  cancel(handle) { clearCallbacks.delete(handle); },
});
clearBatcher.push('discard-on-explicit-clear');
clearBatcher.clear();
assert.equal(clearCallbacks.size, 0, 'clear cancels the pending visual callback');
assert.equal(clearBatcher.flush(), false);
assert.deepEqual(cleared, []);

assert.equal(textDeltaFromMessage({ type: 'text_delta', content: 'direct' }), 'direct');
assert.equal(textDeltaFromMessage({
  type: 'broadcast', event: { TextDelta: { delta: 'mirror' } },
}), 'mirror');
assert.equal(textDeltaFromMessage({ type: 'response', content: 'done' }), null);
assert.equal(isScrollNearBottom(1000, 500, 300), true);
assert.equal(isScrollNearBottom(1000, 100, 300), false);

console.log('Control chat performance contract passed: exact ordered deltas, one commit per 34 ms burst.');
