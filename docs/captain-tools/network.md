# Network family

> **Status:** audited (D.3).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::NETWORK_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### `web_citation_audit`

Deterministic final gate for sourced research. Call it after drafting, with the
draft plus the exact source URLs and verbatim evidence quotes copied from pages
that `web_research_batch` marked `citation_ready`. It refetches up to twelve
independent sources concurrently and checks:

- every inline Markdown citation URL belongs to the submitted source set;
- each cited page still returns a successful non-empty response;
- submitted evidence quotes occur in the retrieved page text after harmless
  whitespace/Markdown normalization;
- a configurable share of prose sentences declares provenance through an
  inline citation or `[unverified]`;
- the canonical `Sources` block is rendered from audited URLs instead of being
  reconstructed from model memory.

| Field | Required | Notes |
|---|---|---|
| `draft` | yes | Up to 50,000 bytes. Put the exact Markdown source link immediately after the supported sentence. |
| `sources` | yes | 1-12 `{url,title?,quotes?}` records. URLs must come from retrieved evidence; max three quotes and 1,000 characters per quote. |
| `min_coverage` | no | `0..1`, default `0.5`. Counts cited and explicitly `[unverified]` prose sentences. |
| `require_evidence` | no | Default `true`; every cited source must carry at least one quote found verbatim in the refetched page. |

`valid=true` certifies URL identity, successful retrieval, declared provenance,
and verbatim evidence presence. It deliberately does **not** claim semantic
entailment or factual truth: Captain must still compare the claim with the
quote, disclose contradictions, and retain `[unverified]` on unsupported
load-bearing claims. The tool is a sequential phase boundary because it depends
on a completed source set and draft; only its independent source fetches run in
parallel.

### `web_research_batch`

Grouped research rail: runs up to five independent `web_search` queries in
parallel, deduplicates canonical URLs, then fetches up to ten selected pages in
parallel. Explicit `urls` are always considered for retrieval; `auto_fetch=false`
keeps discovered search hits as discovery-only candidates.

Each source receives a stable URL-derived `source_id`, discovery queries and
providers, canonical and final URLs, HTTP status, retrieval timestamp, retained
content SHA-256, bounded preview and `citation_markdown`. The latter exists only
when a successful non-empty page was actually read. Search snippets and
provider-generated summaries remain explicitly `discovery_only` and are never
citation-ready. The coverage block reports fetched/failed sources and independent
domains without pretending that successful HTTP retrieval proves a claim.

Use individual `web_fetch` only when an exact page needs a deeper second pass.
For important, disputed, high-stakes, or explicitly fact-checked output, finish
with `web_citation_audit`. For PDF/report/dataset links, use `web_download` and
then `document_extract`; do not cite a binary document from the URL alone.

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

`GET` responses on `text/html` are converted to readable Markdown automatically.
The response exposes HTTP status, final URL, retrieval time and the SHA-256 of
the retained content before the untrusted-content body. Other methods and
content types pass through as raw strings.

For a local API — the daemon itself, MCP servers on `127.0.0.1`, … — use **`shell_exec` with `curl`** instead: `web_fetch` blocks loopback by design.

### `web_search`

Multi-provider web search (Tavily → Brave → Perplexity → DuckDuckGo) with automatic failover.

| Field | Required | Notes |
|---|---|---|
| `query` | yes | Natural-language or keyword query (`"meilleure lib Rust pour HTTP async 2025"`). |
| `max_results` | no | Default 5, capped at 20. |

Each result carries `source_id`, title, original/canonical URL and snippet in a
typed evidence envelope. Every result is marked `discovery_only` and
`citation_ready=false`. Use this to find URLs or vet recent docs; fetch the page
before citing it.

## Sandbox

- **SSRF allowlist** — `web_fetch` rejects URLs whose resolved IP is loopback (`127.0.0.0/8`, `::1`), link-local, RFC1918 (`10/8`, `172.16/12`, `192.168/16`), CGNAT (`100.64/10`), or any other IETF "special-use" range. The check happens **after** DNS resolution so a hostname that resolves to a private IP is also blocked.
- **Scheme allowlist** — only `http` and `https`. `file://`, `gopher://`, `ftp://`, `dict://` are rejected so a redirect cannot pivot to local files.
- **Provider keys** — each search provider reads its API key from `~/.captain/secrets.env` at daemon boot. Keys are not exposed to the LLM and rotation runs through `secret_write` + `channel_reconfigure` is not required (no in-process bridge).
- **Outbound only** — none of these tools open a listening socket. Inbound traffic to Captain only enters through the configured API listener (B.5 governs auth).

## Limites

- `web_fetch` and `web_download` follow redirects (default 10). Each hop is re-validated against the SSRF allowlist; a redirect chain that lands on `169.254.169.254` (cloud metadata) is rejected mid-chain.
- Response body cap: 10 MB by default. The limit is enforced both from `Content-Length` and after reading a response whose length was unknown. Larger payloads return `"response too large"`; use `web_download` for a bounded source artifact.
- `web_download` is for larger source files and defaults to 25 MB with a hard cap of 100 MB. It writes only inside the workspace sandbox and refuses overwrite by default.
- Default request timeout: 30 s. There is no per-tool override; long-running fetches must use `process_start` with `curl --max-time`.
- `web_fetch` does **not** retry on 5xx by itself. Wrap it in your own retry logic only when the API documents that a retry is safe (idempotent verbs, idempotency tokens, …).
- `web_search` returns at most 20 results — for paginated discovery, run several queries with refined keywords rather than asking for more.
- An explicitly selected provider fails closed on its own error. `search_provider = "auto"` tries configured Tavily, Brave and Perplexity providers in order, then the zero-config DuckDuckGo fallback; each fallback is logged without exposing credentials.

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
→ {"results":[{"source_id":"SRC-...","url":"https://docs.rs/ratatui/...","retrieval_status":"discovery_only","citation_ready":false}, ...]}
web_fetch({"url": "https://docs.rs/ratatui/..."})
→ HTTP status + final URL + retrieval time + SHA-256 + Markdown-converted page body.
```

### Grounded research then audit

```
web_research_batch({"queries":["primary specification", "independent analysis"], "max_fetches":5})
→ sources include only fetched pages with citation_ready=true and exact citation_markdown values

web_citation_audit({
  "draft":"The supported claim.[Primary source](https://example.com/spec)",
  "sources":[{
    "url":"https://example.com/spec",
    "title":"Primary source",
    "quotes":["Exact supporting sentence copied from the retrieved page."]
  }]
})
→ {"valid":true,"sources_markdown":"## Sources\n...","scope":"..."}
```

### Error case — SSRF block on a private IP

```
web_fetch({"url": "http://169.254.169.254/latest/meta-data/iam/security-credentials/"})
→ Err("SSRF blocked: 169.254.169.254 is in the link-local allowlist-deny range").
```

The block is the contract — the request never left the daemon.
