import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';
import { CAPABILITY_TABS, capabilityTabForRoute } from '../control_contract.mjs';

const html = htm.bind(h);
const PAGE_SIZE = 40;

export function Capabilities({ route }) {
  const current = capabilityTabForRoute(route);

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Capabilities</h1>
        <p class="page-sub">Skills chargées à la demande et outils disponibles dans le runtime.</p>
        <div class="hub-tabs">
          ${CAPABILITY_TABS.map((tab) => html`
            <a key=${tab.route} class="hub-tab ${tab.route === current.route ? 'active' : ''}"
              href="#/${tab.route}">${tab.label}</a>
          `)}
        </div>
        ${current.route === 'tools' ? html`<${ToolsTab} />` : html`<${SkillsTab} />`}
      </div>
    </div>
  `;
}

function sourceLabel(source) {
  if (!source || !source.type) return '';
  if (source.type === 'bundled') return 'intégrée';
  if (source.type === 'clawhub' || source.type === 'openclaw') return 'importée';
  return 'locale';
}

function SkillsTab() {
  const [data, setData] = useState(null);
  const [query, setQuery] = useState('');
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);

  const load = useCallback(async () => {
    try { setData(await api.skills()); }
    catch (e) { toast(`Chargement impossible : ${e.message}`, 'err'); }
  }, []);

  useEffect(() => { load(); }, [load]);
  useEffect(() => { setVisibleCount(PAGE_SIZE); }, [query]);

  const skills = (data && data.skills) || [];
  const enabled = skills.filter((skill) => skill.enabled !== false).length;
  const filtered = filterItems(skills, query, (skill) => `${skill.name || ''} ${skill.description || ''}`);
  const visible = filtered.slice(0, visibleCount);

  return html`
    <div>
      ${data === null && html`<div class="skeleton" style="height:70px"></div>`}
      ${data && html`
        <div class="capability-toolbar">
          <div class="metrics-row">
            <div class="metric-chip">${data.total ?? skills.length} skill(s)</div>
            <div class="metric-chip ${enabled > 0 ? 'ok' : ''}">${enabled} active(s)</div>
          </div>
          <input type="text" placeholder="Filtrer les skills" value=${query}
            onInput=${(event) => setQuery(event.target.value)} />
        </div>
      `}
      ${data && skills.length === 0 && html`
        <div class="empty-state"><div class="glyph">◇</div><div>Aucune skill installée.</div></div>
      `}
      ${data && skills.length > 0 && filtered.length === 0 && html`<div class="status-empty">Aucun résultat.</div>`}
      ${visible.length > 0 && html`
        <div class="item-list">
          ${visible.map((skill) => html`
            <div class="item-row" key=${skill.name}>
              <div class="item-row-main">
                <strong>${skill.name}</strong>
                ${skill.description && html`<span class="item-row-meta">${skill.description}</span>`}
                <span class="item-row-meta">
                  ${skill.version ? `v${skill.version}` : ''}
                  ${skill.runtime ? ` · ${skill.runtime}` : ''}
                  ${skill.tools_count ? ` · ${skill.tools_count} outil(s)` : ''}
                  ${sourceLabel(skill.source) ? ` · ${sourceLabel(skill.source)}` : ''}
                </span>
              </div>
              <div class="item-row-actions">
                <span class="status-pill ${skill.enabled !== false ? 'status-done' : 'status-cancelled'}">
                  ${skill.enabled !== false ? 'active' : 'inactive'}
                </span>
              </div>
            </div>
          `)}
        </div>
      `}
      ${visible.length < filtered.length && html`
        <button class="ghost capability-more" onClick=${() => setVisibleCount((count) => count + PAGE_SIZE)}>
          Afficher ${Math.min(PAGE_SIZE, filtered.length - visible.length)} de plus
        </button>
      `}
    </div>
  `;
}

function ToolsTab() {
  const [data, setData] = useState(null);
  const [query, setQuery] = useState('');
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);

  const load = useCallback(async () => {
    try { setData(await api.tools()); }
    catch (e) { toast(`Chargement impossible : ${e.message}`, 'err'); }
  }, []);

  useEffect(() => { load(); }, [load]);
  useEffect(() => { setVisibleCount(PAGE_SIZE); }, [query]);

  const tools = ((data && data.tools) || []).slice().sort((left, right) =>
    String(left.name || '').localeCompare(String(right.name || '')));
  const filtered = filterItems(tools, query, (tool) => `${tool.name || ''} ${tool.description || ''} ${tool.source || ''}`);
  const visible = filtered.slice(0, visibleCount);
  const mcpCount = tools.filter((tool) => tool.source === 'mcp').length;

  return html`
    <div>
      ${data === null && html`<div class="skeleton" style="height:70px"></div>`}
      ${data && html`
        <div class="capability-toolbar">
          <div class="metrics-row">
            <div class="metric-chip">${data.total ?? tools.length} outil(s)</div>
            <div class="metric-chip ${mcpCount > 0 ? 'ok' : ''}">${mcpCount} MCP</div>
          </div>
          <input type="text" placeholder="Filtrer les outils" value=${query}
            onInput=${(event) => setQuery(event.target.value)} />
        </div>
      `}
      ${data && tools.length === 0 && html`
        <div class="empty-state"><div class="glyph">⌁</div><div>Aucun outil disponible.</div></div>
      `}
      ${data && tools.length > 0 && filtered.length === 0 && html`<div class="status-empty">Aucun résultat.</div>`}
      ${visible.length > 0 && html`
        <div class="item-list">
          ${visible.map((tool) => html`
            <div class="item-row" key=${tool.name}>
              <div class="item-row-main">
                <strong class="tool-catalog-name">${tool.name}</strong>
                ${tool.description && html`<span class="item-row-meta">${tool.description}</span>`}
              </div>
              <div class="item-row-actions">
                <span class="status-pill ${tool.source === 'mcp' ? 'status-review' : 'status-done'}">
                  ${tool.source === 'mcp' ? 'MCP' : 'builtin'}
                </span>
              </div>
            </div>
          `)}
        </div>
      `}
      ${visible.length < filtered.length && html`
        <button class="ghost capability-more" onClick=${() => setVisibleCount((count) => count + PAGE_SIZE)}>
          Afficher ${Math.min(PAGE_SIZE, filtered.length - visible.length)} de plus
        </button>
      `}
    </div>
  `;
}

function filterItems(items, query, textFor) {
  const needle = query.trim().toLocaleLowerCase('fr');
  if (!needle) return items;
  return items.filter((item) => textFor(item).toLocaleLowerCase('fr').includes(needle));
}
