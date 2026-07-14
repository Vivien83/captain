# Network family

> **Status:** audited (D.3).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::NETWORK_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### `web_research_batch`

Grouped research rail: runs up to five `web_search` queries, extracts result
URLs, and fetches a bounded set of pages in the same tool call. It requires at
least one non-empty `queries` entry; explicit `urls` can be added to force-fetch
known sources alongside the discovered results. It returns compact previews plus
URLs so the agent can synthesize with sources without spending separate
search/fetch turns. Use individual `web_fetch` only when an exact page needs a
deeper second pass. For PDF/report/dataset links, use `web_download` and then
`document_extract`; do not cite a binary document from the URL alone.

### `web_download`

Download an external source file into the agent workspace with the same SSRF
guard philosophy as `web_fetch`. This is the native rail for PDF reports,
CSV/JSON datasets, whitepapers and files that need a local path before a
follow-up tool can inspect them.

| Field | Required | Notes |
|---|---|---|
| `url` | yes | `http://` or `https://` external URL. Redirects are re-validated before following. |
| `path` | no | Workspace-relative output path. Default: `downloads/<detected-filename>`. |
| `max_bytes` | no | Default 25 MB, hard cap 100 MB. |
| `overwrite` | no | Default false; existing files are protected. |

Returns JSON with the final URL, local `path`, MIME type, size, SHA-256, redirect
chain and a `next_action` hint. For text-like files and PDFs, the next step is
normally `document_extract`.

### `web_fetch`

Outbound HTTP request with anti-SSRF protection. The default for talking to a public REST API or grabbing a URL.

| Field | Required | Notes |
|---|---|---|
| `url` | yes | `http://` or `https://` only. Private/loopback IPs are rejected upstream of the request. |
| `method` | no | `GET` (default), `POST`, `PUT`, `PATCH`, `DELETE`. |
| `headers` | no | Object map; common ones: `Authorization`, `Content-Type`, `User-Agent`. |
| `body` | no | String body for `POST`/`PUT`/`PATCH`. JSON, form-encoded, or raw — Captain decides. |

`GET` responses on `text/html` are converted to readable Markdown automatically. Other methods and content types pass through as raw bytes/strings.

For a local API — the daemon itself, MCP servers on `127.0.0.1`, … — use **`shell_exec` with `curl`** instead: `web_fetch` blocks loopback by design.

### `web_search`

Multi-provider web search (Tavily → Brave → Perplexity → DuckDuckGo) with automatic failover.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Natural-language or keyword query (`"meilleure lib Rust pour HTTP async 2025"`). |
| `max_results` | no | Default 5, capped at 20. |

Each result is `{title, url, snippet}`. Use this to find URLs, vet recent docs, or sanity-check a fact; once you have a URL feed it back through `web_fetch`.

## Sandbox

- **SSRF allowlist** — `web_fetch` rejects URLs whose resolved IP is loopback (`127.0.0.0/8`, `::1`), link-local, RFC1918 (`10/8`, `172.16/12`, `192.168/16`), CGNAT (`100.64/10`), or any other IETF "special-use" range. The check happens **after** DNS resolution so a hostname that resolves to a private IP is also blocked.
- **Scheme allowlist** — only `http` and `https`. `file://`, `gopher://`, `ftp://`, `dict://` are rejected so a redirect cannot pivot to local files.
- **Provider keys** — each search provider reads its API key from `~/.captain/secrets.env` at daemon boot. Keys are not exposed to the LLM and rotation runs through `secret_write` + `channel_reconfigure` is not required (no in-process bridge).
- **Outbound only** — none of these tools open a listening socket. Inbound traffic to Captain only enters through the configured API listener (B.5 governs auth).

## Limites

- `web_fetch` and `web_download` follow redirects (default 10). Each hop is re-validated against the SSRF allowlist; a redirect chain that lands on `169.254.169.254` (cloud metadata) is rejected mid-chain.
- Response body cap: 5 MB. Larger payloads return `"response too large"` rather than streaming — fall back to `shell_exec` + `curl -o` if you genuinely need a big file.
- `web_download` is for larger source files and defaults to 25 MB with a hard cap of 100 MB. It writes only inside the workspace sandbox and refuses overwrite by default.
- Default request timeout: 30 s. There is no per-tool override; long-running fetches must use `process_start` with `curl --max-time`.
- `web_fetch` does **not** retry on 5xx by itself. Wrap it in your own retry logic only when the API documents that a retry is safe (idempotent verbs, idempotency tokens, …).
- `web_search` returns at most 20 results — for paginated discovery, run several queries with refined keywords rather than asking for more.
- All providers fail closed: when the configured API key is missing or invalid the tool returns the upstream error; it does not silently skip to the next provider unless the failure mode is `429` / `503`.

## Exemples

### Golden path — fetch a public API and parse JSON

```
web_fetch({
  "url": "https://api.github.com/repos/anthropics/claude-code/releases/latest",
  "headers": {"User-Agent": "captain/3"}
})
→ {"status": 200, "body": "{\"tag_name\":\"v1.4.2\", …}"}
```

### Golden path — search then fetch

```
web_search({"query": "ratatui mouse capture example", "max_results": 3})
→ [{"title": "...", "url": "https://docs.rs/ratatui/...", "snippet": "..."}, ...]
web_fetch({"url": "https://docs.rs/ratatui/..."})
→ Markdown-converted page body.
```

### Error case — SSRF block on a private IP

```
web_fetch({"url": "http://169.254.169.254/latest/meta-data/iam/security-credentials/"})
→ Err("SSRF blocked: 169.254.169.254 is in the link-local allowlist-deny range").
```

The block is the contract — the request never left the daemon.
