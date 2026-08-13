import { h } from '/assets/app/vendor/preact.module.js';
import { useState, useEffect } from '/assets/app/vendor/hooks.module.js';
import htm from '/assets/app/vendor/htm.module.js';
import { getState, subscribe } from '../store.js';
import { Approvals } from './Approvals.js';
import { Triggers } from './Triggers.js';
import { Crons } from './Crons.js';
import { Webhooks } from './Webhooks.js';
import { Workflows } from './Workflows.js';
import { automationTabsForMode, automationTabForRoute } from '../control_contract.mjs';

const html = htm.bind(h);

const VIEWS = {
  workflows: Workflows,
  triggers: Triggers,
  crons: Crons,
  approvals: Approvals,
  webhooks: Webhooks,
};

export function Automation({ route }) {
  const [state, setLocalState] = useState({
    approvalsCount: getState().approvalsCount,
    clientMode: getState().clientMode,
  });
  useEffect(() => subscribe((next) => setLocalState({
    approvalsCount: next.approvalsCount,
    clientMode: next.clientMode,
  })), []);

  const tabs = automationTabsForMode(state.clientMode);
  const current = automationTabForRoute(route, state.clientMode);
  const View = VIEWS[current.route];

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Automation</h1>
        <p class="page-sub">${state.clientMode
          ? 'Workflows partagés et décisions humaines du Hub.'
          : 'Workflows, déclenchements et livraisons automatisées.'}</p>
        <div class="hub-tabs">
          ${tabs.map((t) => html`
            <a key=${t.route} class="hub-tab ${t.route === current.route ? 'active' : ''}" href="#/${t.route}">
              ${t.label}
              ${t.route === 'approvals' && state.approvalsCount > 0 && html`<span class="badge">${state.approvalsCount}</span>`}
            </a>
          `)}
        </div>
        <${View} clientMode=${state.clientMode} />
      </div>
    </div>
  `;
}
