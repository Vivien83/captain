import { h } from 'preact';
import { useState } from 'preact/hooks';
import htm from 'htm';

const html = htm.bind(h);

// A tool call inside the transcript: running → ok/error, collapsible details.
// `tool` shape: { id, name, input, result, isError, done, startedAt, endedAt }
export function ToolCard({ tool }) {
  const [open, setOpen] = useState(false);
  const cls = tool.done ? (tool.isError ? 'error' : 'ok') : 'running';
  const duration = tool.endedAt && tool.startedAt
    ? `${((tool.endedAt - tool.startedAt) / 1000).toFixed(1)}s`
    : null;

  return html`
    <div class="tool-card ${cls} ${open ? 'open' : ''}">
      <div class="head" onClick=${() => setOpen(!open)}>
        <span class="chevron">▶</span>
        <span class="tool-name">${tool.name}</span>
        <span class="state">
          ${!tool.done && html`<span class="spinner"></span>`}
          ${!tool.done && 'en cours'}
          ${tool.done && (tool.isError ? '✗ erreur' : '✓ terminé')}
          ${duration && html`<span>· ${duration}</span>`}
        </span>
      </div>
      ${open && html`
        <div class="body">
          ${tool.input && html`<div><div class="label">Entrée</div>${tool.input}</div>`}
          ${tool.result && html`<div><div class="label">Résultat</div>${tool.result}</div>`}
          ${!tool.input && !tool.result && html`<div>—</div>`}
        </div>
      `}
    </div>
  `;
}
