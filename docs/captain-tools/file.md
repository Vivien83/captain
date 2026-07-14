# File family

> **Status:** audited (D.1).
> See [`README.md`](README.md) for the index and drift policy.
> Tool name list pinned in [`captain_runtime::captain_docs::FILE_FAMILY_TOOLS`](../../crates/captain-runtime/src/captain_docs.rs).

## Tools

### `file_inspect_batch`

Read-only grouped inspection: runs multiple `glob`, `grep`, `read`, or `list`
operations in one tool call. Use it for repo/document audits where separate
file calls would waste context. `read` outputs are truncated by
`max_read_chars`; write/edit actions are intentionally unsupported.

### `file_read`

Read the UTF-8 content of a workspace file.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Workspace-relative path. Absolute paths are resolved through the multi-root sandbox (Captain only). |

Returns the raw string. Use **`file_list`** to inspect a directory and **`glob`** for pattern matching — `file_read` is for known files only.

### `file_write`

Create or fully overwrite a file. **Destructive on existing files.**

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Parents are auto-created. |
| `content` | yes | Replaces the entire file. |

Reach for **`edit_file`** to change a line in an existing file, **`multi_edit`** for several atomic substitutions, **`apply_patch`** for hunked diffs. `file_write` only when creating a brand-new file or rewriting one wholesale.

All write/edit tools refuse newly-added content that looks like a raw secret. If the user gives an API key, store it with `secret_write`; generated files may reference an env var name such as `GEMINI_API_KEY`, never the literal key.

### `file_list`

Single-level directory listing (does **not** recurse). Returns names tagged with `file` or `dir`.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | `.` for the workspace root. |

For recursive walks or pattern filters use **`glob`**.

### `glob`

Recursive, gitignore-aware filename match. Backed by ripgrep's walker — never shell out to `find`.

| Field | Required | Notes |
|---|---|---|
| `pattern` | yes | `*.rs`, `src/**/*.{ts,tsx}`, `**/CHANGELOG*`. |
| `path` | no | Search root, defaults to `.`. |
| `head_limit` | no | Result cap, default 1000. |

Results are sorted by mtime descending so the freshest files surface first.

### `grep`

Recursive content search across the workspace, ripgrep semantics, embedded (no shell out to `rg`).

| Field | Required | Notes |
|---|---|---|
| `pattern` | yes | Regex. |
| `path` | no | Search root, defaults to `.`. |
| `glob` / `type` | no | Filename filter (`*.rs`, alias `rust`/`ts`/`py`/...). |
| `output_mode` | no | `files_with_matches` (default), `content`, `count`. |
| `-A` / `-B` / `-C` | no | Context lines, only with `output_mode=content`. |
| `-i` | no | Case-insensitive. |
| `multiline` | no | `.` matches `\n`, regex spans lines. |
| `head_limit` | no | Result cap, default 250. |

Skips files >5 MB and binaries automatically.

### `edit_file`

`str_replace`-style targeted edit on an existing file. **Default choice for modifying an existing file** — safer than `file_write`, simpler than `apply_patch`.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Workspace-relative. |
| `old_string` | yes | Must match a unique slice unless `replace_all=true`. |
| `new_string` | yes | May be empty (deletion). |
| `replace_all` | no | Default `false`. |

Eight fallback strategies (whitespace-tolerant, indentation-aware, anchor-based) are attempted before failing — the response includes which strategy matched.

### `multi_edit`

Atomic chain of `edit_file` substitutions on the same file. Either all edits land or none — partial writes are impossible.

| Field | Required | Notes |
|---|---|---|
| `path` | yes | Workspace-relative. |
| `edits` | yes | Array of `{old_string, new_string, replace_all?}`. |

Each edit sees the result of the previous one in memory; the disk write happens after the last edit succeeds. Use this when a coherent set of changes must apply together (rename a symbol + update its callsites in one file, for example).

### `apply_patch`

Multi-hunk unified-diff patcher. Add, modify, move or delete files surgically.

| Field | Required | Notes |
|---|---|---|
| `patch` | yes | Wrapped in `*** Begin Patch` / `*** End Patch`, with section markers `*** Add File:`, `*** Update File:`, `*** Delete File:`. |

Returns the list of files touched and hunks applied. Use **`file_write`** to create a new file from scratch — `apply_patch` is overkill for that, and the patch grammar is unforgiving on adds.

## Sandbox

- All paths are resolved through `resolve_sandbox_path` against the agent's workspace root, plus any extra roots granted by `workspace_add` (Captain only). The blocklist (`~/.ssh`, `~/.gnupg`, raw Captain credential stores) overrides every grant.
- Path-traversal components (`..`) are rejected before any filesystem call (`validate_path`).
- Symlink escapes are blocked — the canonicalized target must stay inside an allowed root, otherwise the call fails before the read/write.
- `glob` and `grep` honour `.gitignore` by default; this is not a security boundary but a noise filter.

## Limites

- `file_read` decodes UTF-8 lossily — binary files return mojibake; use `shell_exec` + `xxd` if you really need bytes.
- `file_write` truncates atomically (write to temp + rename). It does not call `fsync` between the two; a kernel crash mid-write can leave the temp file behind.
- `file_write`, `edit_file`, `multi_edit` and `apply_patch` block raw API-key/token/password literals in content they would add. This is a security feedback loop for the model: use the vault + `env_inject` instead of embedding secrets.
- `file_list` does **not** recurse. Use `glob` if you need depth.
- `glob` caps results at `head_limit` (default 1000) — large monorepos may hit it; tighten the pattern before bumping the cap.
- `grep` caps at `head_limit` (default 250) and skips files >5 MB or detected as binary; for those, fall back to `shell_exec` with `rg --no-ignore`.
- `edit_file` requires the `old_string` to be unique unless `replace_all=true`. Eight fallback strategies are tried; if all fail, the call errors out without writing — the file is never half-edited.
- `multi_edit` is atomic per-file. Cross-file atomicity does not exist; use `apply_patch` for that.
- `apply_patch` requires exact context lines for hunks. A drift between the patch and the file leads to a hunk failure and no file is touched.

## Exemples

### Golden path — read then targeted edit

```
1. file_read({"path": "config/settings.toml"})
   → returns the TOML body.
2. edit_file({
     "path": "config/settings.toml",
     "old_string": "log_level = \"info\"",
     "new_string": "log_level = \"debug\""
   })
   → "edit applied via strategy=Exact, replacements=1".
```

### Error case — refusing a path outside the workspace

```
file_write({
  "path": "../../etc/passwd",
  "content": "x"
})
→ Err("Path traversal not allowed: '..' component in path").
```

This error is the contract — the file system was never touched.
