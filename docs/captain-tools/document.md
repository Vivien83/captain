# Document Family

> **Status:** audited (D.19).
> See [`README.md`](README.md) for the index and drift policy.

This family creates workspace documents and promotes final files into Captain's
immutable artifact store when they must remain inspectable, downloadable or
deliverable after the current turn.

## Tools

### `document_extract`

Extract text from a workspace document so Captain can summarize, compare, verify
and cite it without guessing from the filename or URL. Typical flow:
`web_download` a PDF/report/dataset, then `document_extract` the saved path.

Supported input:

- PDF with embedded text streams (no OCR yet).
- Text-like files: TXT, Markdown, HTML, CSV, JSON, XML, logs and source files.

Input:

- `path`: workspace-relative source file path.
- `max_chars`: optional output cap; default 50k chars, hard cap 200k.

The tool returns JSON with `format`, byte size, extracted character count,
truncation status, extraction metadata and `content`. If a PDF is scanned or
image-only, it returns an explicit no-text error; the agent must switch to OCR,
vision, or another source instead of inventing.

### `document_pipeline`

Create a document with `document_create` and optionally deliver it through
`channel_send` in the same tool call. Use it for polished reports or research
briefs that should be sent to Telegram, Discord, email, or another configured
channel after rendering.

Input:

- `document`: full `document_create` input.
- `send`: optional `channel_send` input without `file_path`; Captain attaches
  the generated document path automatically.

### `document_create`

Generate a document inside the workspace.

If `content` starts with a level-1 Markdown heading identical to `title`,
Captain treats it as the document title and removes that duplicate body heading
before rendering. This keeps generated reports from starting with the same title
twice.

Formats:

- `pdf`
- `docx`
- `html`
- `markdown`

Input styles:

- `content`: Markdown-like text with headings (`#`, `##`), paragraphs, bullets
  (`- item`) and pipe tables.
- `sections`: structured blocks with `heading`, `body`, `bullets` and `table`.
- `citations`: source records appended at the end of the document.

The tool returns JSON with `path`, `format`, `mime_type`, `size_bytes`, optional
`pages`, and a `next_action` hint for sending the artifact.

### `artifact_publish`

Copy one final workspace file into Captain's crash-safe immutable store. A new
artifact starts at version 1; passing an existing `artifact_id` owned by the same
agent creates its next version without replacing earlier payloads.

Input:

- `path` and `title`: required;
- `artifact_id`: optional UUID for a new version;
- `filename`, `mime_type`, `summary`: optional presentation metadata.

The tool scans UTF-8 content and metadata for literal secrets, computes the
inspected SHA-256, and requires the store copy to match that digest. A source
changed between inspection and copy is rejected. The 50 MiB per-file limit and
the store quota fail closed without pruning prior versions.

### `artifact_list` and `artifact_inspect`

`artifact_list` returns only the calling agent's latest versions, with a maximum
of 100 items. `artifact_inspect` selects an exact version, or the latest when
`version` is omitted, and verifies payload size plus SHA-256 before returning
metadata. These are reviewed read-only tools and independent calls may run in
parallel.

### `artifact_deliver`

Upload a checksum-verified owned artifact through an active channel without
exposing its managed local path. Images use the native image upload with a
caption; other formats use the native file upload. `recipient` may be omitted
only when the channel has a configured default.

Publication and delivery are external or durable effects. They remain
sequential fences: first wait for `artifact_publish`, then pass its returned
`artifact_id` and `version` to `artifact_deliver`.

## Action

Use `document_create` when the user asks for a PDF, DOCX, report, synthesis,
research brief, memo, invoice, technical sheet, meeting notes, or any polished
document artifact.

Prefer this tool over `file_write` for user-facing deliverables because it owns:

- output format selection,
- default path generation under `documents/`,
- overwrite protection,
- document structure,
- source appendix,
- binary PDF/DOCX writing.

Use `file_write` only for raw notes, source files, templates, or intermediate
Markdown that the user did not ask to receive as a finished document.

Use `document_extract` before citing or summarizing a downloaded PDF/report.
The final answer or generated document must cite the original source URL/path
actually read, not just the search result that discovered it.

For a one-turn disposable send, `document_pipeline` remains available. For a
deliverable that must survive restart and be reopened from another surface, use
`document_create` -> `artifact_publish` -> `artifact_deliver` in that order.

## Sandbox

`path` resolves through the workspace sandbox. Relative paths are rooted in the
agent workspace. `..` traversal is rejected. Parent directories are created
automatically.

The tool refuses to overwrite an existing file unless `overwrite=true` is
explicitly provided.

Like other write tools, `document_create` rejects obvious raw secrets in title,
content, sections and citations. Store secrets with `secret_write` and reference
the secret name instead of embedding values in a document.

The immutable store is kernel-owned. Agent tools never receive its payload path,
cannot read another agent's artifacts, and cannot forge the owning agent or
session. Unknown active formats, including SVG, remain download-only until a
sandboxed preview explicitly supports them.

## Limites

The base renderer is intentionally dependency-free. It is reliable for clean
reports, summaries, tables, invoices, memos and handoff documents.

Known limits:

- PDF uses built-in Helvetica/Helvetica-Bold and simple page layout.
- Complex brand layouts, precise typography, charts, covers and PDF/A/PDF/UA
  conformance should move to a dedicated document skill or a future Typst
  backend.
- DOCX output is a compact OpenXML package with core paragraph/table styles,
  not a full Word design system.

Do not claim advanced publishing guarantees unless a richer backend or skill has
actually rendered and verified the file.

## Exemples

Create a research PDF:

```json
{
  "format": "pdf",
  "path": "documents/market-research.pdf",
  "title": "Market Research Summary",
  "subtitle": "Executive brief",
  "content": "# Findings\nThe strongest signal is...\n\n| Source | Signal |\n| --- | --- |\n| Vendor docs | Stable API |",
  "citations": [
    {
      "id": "1",
      "title": "Official vendor documentation",
      "url": "https://example.com/docs",
      "accessed_at": "2026-05-05"
    }
  ]
}
```

Create a DOCX from structured sections:

```json
{
  "format": "docx",
  "title": "Incident Report",
  "sections": [
    {
      "heading": "Summary",
      "body": "The service stayed available.",
      "bullets": ["No restart", "No failed systemd unit"]
    }
  ]
}
```

Publish and send the generated artifact durably:

```json
{
  "path": "documents/market-research.pdf",
  "title": "Market Research Summary"
}
```

Then use the returned identity:

```json
{
  "artifact_id": "9e9afb94-dc24-48a5-a2b8-4f72190411d1",
  "version": 1,
  "channel": "telegram"
}
```
