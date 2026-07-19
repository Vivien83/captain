// Shared navigation contract for Captain Control. Keep route ownership here so
// the shell, router and hub tabs cannot drift independently.

export const PRIMARY_HUBS = Object.freeze([
  { route: 'chat', icon: '💬', label: 'Chat' },
  { route: 'projects', icon: '📁', label: 'Projects' },
  { route: 'automation', icon: '⚡', label: 'Automation' },
  { route: 'learning', icon: '🧠', label: 'Learning' },
  { route: 'capabilities', icon: '🧩', label: 'Capabilities' },
  { route: 'status', icon: '◉', label: 'Status' },
]);

export const AUTOMATION_TABS = Object.freeze([
  { route: 'workflows', label: 'Workflows' },
  { route: 'triggers', label: 'Triggers' },
  { route: 'crons', label: 'Crons' },
  { route: 'approvals', label: 'Approbations' },
  { route: 'webhooks', label: 'Webhooks' },
]);

export const CAPABILITY_TABS = Object.freeze([
  { route: 'native-capabilities', label: 'Natives' },
  { route: 'skills', label: 'Skills' },
  { route: 'tools', label: 'Tools' },
]);

export const ROUTE_HUB = Object.freeze({
  workflows: 'automation',
  triggers: 'automation',
  crons: 'automation',
  approvals: 'automation',
  webhooks: 'automation',
  'native-capabilities': 'capabilities',
  skills: 'capabilities',
  tools: 'capabilities',
  // Frozen Hands links remain non-breaking but resolve to the safe default
  // Capabilities view instead of exposing Hands in active navigation.
  hands: 'capabilities',
  system: 'status',
});

export function hubForRoute(route) {
  return ROUTE_HUB[route] || route || 'chat';
}

export function automationTabForRoute(route) {
  return AUTOMATION_TABS.find((tab) => tab.route === route) || AUTOMATION_TABS[0];
}

export function capabilityTabForRoute(route) {
  return CAPABILITY_TABS.find((tab) => tab.route === route) || CAPABILITY_TABS[0];
}
