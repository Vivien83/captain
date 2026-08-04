# Captain Interaction Performance Budgets

This document defines reproducible rendering budgets for Captain's active chat
surfaces. These are regression contracts, not claims about network or model
latency on every host.

## Shared Invariants

- User input is never delayed to wait for a background rendering window.
- Ordered text deltas remain byte-for-byte ordered across visual batching.
- No text delta may be dropped to satisfy a frame or memory budget.
- A non-text event flushes every earlier text delta before it is rendered.
- An operator who scrolls into history is not forced back to the live tail.
  A transcript already near the tail remains pinned while its DOM grows.

## TUI And Web Terminal

The native Ratatui event loop groups background events for at most 34 ms. Key,
paste, scroll, and mouse events still trigger an immediate frame. One frame may
consume at most 2,048 queued events; remaining events stay ordered in the queue
for the next frame. The 50 ms idle tick keeps animations bounded at 20 frames
per second when no other event exists.

Completed transcript history is cached by history revision, terminal width,
tool-animation state, and mouse-capture mode. The long-history contract parses
200 settled messages once across 100 frames that only change the live tail. A
history mutation or resize must rebuild the cache exactly. The Web terminal and
Desktop terminal inherit this same TUI path.

## Control Web And Desktop

The daemon groups direct WebSocket text for up to 100 ms or 200 characters.
The browser adds a defensive visual batch of at most 34 ms. Its pending buffer
flushes immediately at 32,768 characters, preserving all content. Tool,
response, error, catch-up, and other non-text boundaries synchronously flush
earlier deltas so transport order and visual order cannot diverge.

Restored rows do not replay entry animations. Settled rows use CSS content
containment, while the active streaming row remains fully visible. The
pre-mutation scroll intent is retained through layout so asynchronous DOM
growth cannot be mistaken for an operator scrolling away.

The browser smoke certifies both 1,440 x 900 and 390 x 844 viewports with:

- 240 restored transcript messages hydrated in less than 2,500 ms;
- 1,000 synchronous text deltas rendered exactly and in order;
- no more than four observed DOM mutation batches for that burst;
- no horizontal overflow or element outside the viewport;
- the live tail pinned, with the composer and provider quota bar visible.

The retained Desktop wrapper embeds the same Control assets and therefore uses
the same contract.

## Reproducible Verification

```bash
scripts/core-surface-gates.sh chat
```

The focused deterministic and browser checks are:

```bash
scripts/control-web-audit.sh
scripts/control-xss-smoke.mjs
scripts/control-chat-performance-smoke.mjs
```

The XSS smoke runs the production CSP against malicious Markdown, tool output,
and session labels. Browser smokes may write disposable screenshots below
`/private/tmp` for visual inspection; they do not add generated images to the
repository.
