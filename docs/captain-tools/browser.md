# Browser family

> **Status:** audited (D.4).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::BROWSER_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

The browser tools share a single persistent headless Chrome session per agent (Chrome remote debugging protocol). The session opens lazily on the first `browser_navigate` and stays warm until `browser_close` or the agent loop exits. Every other `browser_*` tool operates on the active page of that session.

### `browser_batch`

Preferred tool for multi-step browser work. It executes up to 20 browser actions in one runtime call, then returns compact per-step summaries plus a final observation.

| Field | Required | Notes |
|---|---|---|
| `steps` | yes | Array of `{action, ...}` items. Actions: `navigate`, `click`, `type`, `keys`, `select`, `hover`, `scroll`, `wait`, `run_js`, `read_page`, `screenshot`, `observe`, `status`, `network_log`, `diagnostics`, `back`, `close`. |
| `stop_on_error` | no | Default `true`. |
| `include_data` | no | Default `false`; raw step data is omitted/truncated to reduce context. |
| `final_observation` | no | `observe` default, or `read_page`, `status`, `diagnostics`, `none`. |
| `max_elements` | no | Max interactive elements for `observe`, default 60, max 120. |

Use this instead of multiple calls for flows like `navigate → wait → observe`, `type → click → diagnostics`, or `scroll → read_page`. Set `final_observation:"read_page"` when the page text is the actual deliverable; keep `observe` for UI interaction.

During a `browser_batch`, Captain emits a live activity timeline (`open`, `click`, `type`, `wait`, `screenshot`, diagnostics, final observation). TUI, web terminal, API streaming, and Telegram all receive the same semantic progress. This is meant for human observability; keep grouped browser actions intentional and readable.

## Search and anti-bot policy

- Use `web_search` / `web_research_batch` for open-ended discovery and source finding.
- Use browser tools for direct URLs, JavaScript-rendered pages, forms, login, downloads that require a page interaction, screenshots, and visual verification.
- Do **not** use headless browser Google search as the default discovery path. It commonly returns `/sorry`, unusual-traffic, CAPTCHA, or automated-query pages.
- If the browser detects CAPTCHA, Google `/sorry`, anti-bot, rate-limit, or human-verification pages, stop retrying that path. Do not solve CAPTCHAs. Switch to native search, Bing/DuckDuckGo, or direct source URLs and mention the block only when it affects the result.

### `browser_navigate`

Open or change page. Always the first call in any browsing flow.

| Field | Required | Notes |
|---|---|---|
| `url` | yes | `http://` or `https://` only — must include the scheme. |

Returns the page title and the body converted to Markdown.

### `browser_click`

Click an element by CSS selector or by visible text.

| Field | Required | Notes |
|---|---|---|
| `selector` | yes | `#submit-btn`, `.add-to-cart`, `button[type=submit]`, or visible text. |

If the element is below the fold, run `browser_scroll` first.

### `browser_type`

Type text into a form field.

| Field | Required | Notes |
|---|---|---|
| `selector` | yes | `input[name=email]`, `#search-box`, … |
| `text` | yes | Exact string to type. |

### `browser_keys`

Send keyboard input to the currently focused element or page.

| Field | Required | Notes |
|---|---|---|
| `keys` | yes | `Enter`, `Tab`, `Escape`, `Backspace`, `ArrowDown`, `Control+a`, `Meta+k`, or literal text for the focused element. |

Prefer this after `browser_type`/`browser_click` when a site expects keyboard submission or navigation. Prefer `browser_type` for normal text entry into a known field.

### `browser_select`

Select an option in a native HTML `<select>` field by value, label, or visible text.

| Field | Required | Notes |
|---|---|---|
| `selector` | yes | CSS selector or `@eN` ref for the `<select>` element. |
| `value` | yes | Option value, label, or exact visible text. |

### `browser_hover`

Move the pointer over an element to reveal menus, popovers, or hover-only controls.

| Field | Required | Notes |
|---|---|---|
| `selector` | yes | CSS selector, visible text, or `@eN` ref. |

Run `browser_observe` after hover if you need the newly revealed controls.

### `browser_screenshot`

PNG capture of the active page, saved to Captain's upload store and returned as `image_urls`. Use this to **verify visually** (CAPTCHA, layout regression, screenshot for the user). For text extraction prefer `browser_read_page`.

No parameters.

### `browser_read_page`

Markdown dump of the current page (title + URL + structured body). Cheaper than `browser_screenshot` when you only need text.

No parameters.

### `browser_scroll`

Move the viewport in one direction.

| Field | Required | Notes |
|---|---|---|
| `direction` | no | `up`, `down` (default), `left`, `right`. |
| `amount` | no | Pixel offset, default 600. |

Combine with `browser_read_page` after scrolling to read the freshly visible content.

### `browser_wait`

Block until a CSS selector appears (AJAX, SPA route change). Returns the page once the selector is present.

| Field | Required | Notes |
|---|---|---|
| `selector` | yes | What to wait for. |
| `timeout_ms` | no | Default 5000, max 30000. |

If the element is already present, **don't wait** — call `browser_read_page` directly.

### `browser_run_js`

Run an arbitrary JavaScript expression in the page context. Returns the JSON-serialized result.

| Field | Required | Notes |
|---|---|---|
| `expression` | yes | One JS expression — `document.querySelectorAll('.x').length`, `JSON.stringify({a: window.foo})`, … |

This is the escape hatch when none of the other browser tools fit (custom DOM walks, structured data extraction, event triggering). Avoid for actions covered by `browser_click` / `browser_type` — those are auditable, `browser_run_js` is not.

### `browser_back`

Navigate to the previous page in history (browser back button). Returns the new page body.

No parameters.

### `browser_status`

Inspect the browser rail without creating a new session. Returns Chrome availability, the isolated profile directory for the current agent, viewport, active session count, and current page metadata when a browser is already open.

No parameters.

Use this when a browsing flow behaves unexpectedly, when you need to verify whether the warm session exists, or when diagnosing profile/cookie isolation.

### `browser_network_log`

Read the recent Chrome DevTools Protocol network journal for the active browser session.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | Number of events to return, default 50, max 200. |
| `clear` | no | If true, clears the journal after reading. |

Returns request, response and loading failure events with URL, HTTP status, MIME type and error text when available. It does **not** create a browser session; call `browser_navigate` first.

### `browser_observe`

Read a compact interaction map of the active page: title, URL, viewport, scroll position, and visible interactive elements.

| Field | Required | Notes |
|---|---|---|
| `max_elements` | no | Default 60, max 120. |

Each element receives a stable ref like `@e1` and a selector like `[data-captain-ref="e1"]`. You can pass `@e1` directly to `browser_click`, `browser_type`, `browser_select`, `browser_hover`, `browser_wait`, or a `browser_batch` step.

### `browser_diagnostics`

One-call diagnostics bundle: `browser_status`, `browser_observe`, recent `browser_network_log`, and recent browser console/page-error events.

| Field | Required | Notes |
|---|---|---|
| `limit` | no | Network/console event count, default 50, max 200. |
| `clear` | no | Clear journals after read, default false. |
| `max_elements` | no | Observation size, default 60, max 120. |

Use this when a dynamic page, web fetch, login, form, or SPA interaction fails. It avoids spending four tool calls on separate status/network/console/page reads.

### `browser_close`

Close the active session and release Chrome resources (memory, ports, file descriptors). Auto-invoked at the end of the agent loop, but call it explicitly when you know you're done.

No parameters.

### `screenshot`

Full-screen capture of the **host desktop** (not the headless browser). Auto-detects the native command: `screencapture` (macOS), `grim` / `gnome-screenshot` / `import` (Linux), `nircmd` (Windows).

| Field | Required | Notes |
|---|---|---|
| `save_path` | no | Defaults to `/tmp/captain_screenshot_<timestamp>.png`. |

Use this when the user wants a snapshot of their actual UI, not the page Captain is browsing.

## Sandbox

- **Per-agent isolation (B.7)** — each agent's Chrome instance runs with its own `--user-data-dir` under `~/.captain/browser-profiles/`, so cookies, localStorage and session tokens stay scoped to one agent.
- **No file:// or chrome:// access** — `browser_navigate` rejects schemes other than `http(s)`. A page can still embed an `<iframe src="file://…">` but the framed content is sandboxed by Chrome itself.
- **No DevTools surface beyond CDP** — `browser_run_js` runs in the page context, not the privileged extension context.
- **env_clear on Chrome spawn** — same B.1 whitelist as every other subprocess. Chrome inherits only `PATH`, `HOME`, temp-dir vars, locale vars and `TERM`; API keys do not propagate to extensions or subprocesses Chrome itself spawns.

## Limites

- One session per agent. Multiple `browser_navigate` calls **replace the active page** instead of opening tabs.
- `browser_screenshot` stores a PNG and returns an upload URL. Large viewports (4K+) still create heavy artifacts — narrow with `browser_scroll` to a region first if you only need a slice.
- `browser_read_page` strips scripts and inline styles; dynamic widgets that hide content via CSS may appear absent. Use `browser_run_js` to reach the live DOM.
- `browser_wait` polls the selector — it will not detect an element that disappears and reappears between polls.
- `browser_run_js` results larger than 5 MB are truncated; serialize a slice (`.slice(0, 1000)`) before stringifying when scraping big DOMs.
- `browser_network_log` is a bounded ring buffer of recent CDP network events. It is for diagnosis, not a full HAR archive; response bodies are not captured.
- `browser_observe` returns visible interactive elements only; hidden menus, virtualized rows, and off-screen controls may require `browser_scroll`, `browser_hover`, click-open, or `browser_run_js`.
- `browser_batch` is bounded to 20 steps and still runs sequentially inside one browser session. It reduces LLM/tool round trips, not page load time.
- `browser_close` is idempotent — calling it twice or on a never-opened session is a no-op, not an error.
- `screenshot` is a separate tool from the headless browser; it captures **whatever is on the user's desktop right now**, not Captain's browser session. Don't confuse the two.

## Exemples

### Golden path — grouped form flow

```
browser_batch({
  "steps": [
    {"action": "navigate", "url": "https://example.com/login"},
    {"action": "type", "selector": "#email", "text": "alice@example.com"},
    {"action": "type", "selector": "#password", "text": "••••••••"},
    {"action": "keys", "keys": "Enter"},
    {"action": "wait", "selector": ".app-shell", "timeout_ms": 10000}
  ],
  "final_observation": "diagnostics"
})
→ {"success": true, "steps_executed": 5, "final_observation": {"status": ..., "observation": ..., "network": ..., "console": ...}}
```

### Ref workflow — observe then click

```
browser_batch({
  "steps": [
    {"action": "navigate", "url": "https://example.com"},
    {"action": "observe"}
  ],
  "final_observation": "observe"
})
→ elements: [{"ref":"@e1","role":"link","text":"Pricing"}, ...]

browser_batch({
  "steps": [
    {"action": "click", "selector": "@e1"},
    {"action": "wait", "selector": "main"}
  ],
  "final_observation": "read_page"
})
```

### Error case — a wrong scheme is rejected

```
browser_navigate({"url": "file:///etc/passwd"})
→ Err("scheme not allowed: only http and https are accepted").
```

This rejection happens before Chrome is told about the URL — there is no DevTools log entry, no network attempt.
