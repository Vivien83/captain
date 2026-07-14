import { h } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import htm from 'htm';
import { getState, subscribe } from '../store.js';
import { Approvals } from './Approvals.js';
import { Triggers } from './Triggers.js';
import { Crons } from './Crons.js';
import { Webhooks } from './Webhooks.js';
import { Workflows } from './Workflows.js';
import { AUTOMATION_TABS, automationTabForRoute } from '../control_contract.mjs';

const html = htm.bind(h);

const VIEWS = {
  workflows: Workflows,
  triggers: Triggers,
  crons: Crons,
  approvals: Approvals,
  webhooks: Webhooks,
};

export function Automation({ route }) {
  const [count, setCount] = useState(getState().approvalsCount);
  useEffect(() => subscribe((s) => setCount(s.approvalsCount)), []);

  const current = automationTabForRoute(route);
  const View = VIEWS[current.route];

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Automation</h1>
        <p class="page-sub">Workflows, déclenchements et livraisons automatisées.</p>
        <div class="hub-tabs">
          ${AUTOMATION_TABS.map((t) => html`
            <a key=${t.route} class="hub-tab ${t.route === current.route ? 'active' : ''}" href="#/${t.route}">
              ${t.label}
              ${t.route === 'approvals' && count > 0 && html`<span class="badge">${count}</span>`}
            </a>
          `)}
        </div>
        <${View} />
      </div>
    </div>
  `;
}
