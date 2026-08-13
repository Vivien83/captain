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

export const CLIENT_PRIMARY_HUBS = Object.freeze(
  PRIMARY_HUBS.filter(({ route }) => ['chat', 'projects', 'automation', 'status'].includes(route)),
);

export const AUTOMATION_TABS = Object.freeze([
  { route: 'workflows', label: 'Workflows' },
  { route: 'triggers', label: 'Triggers' },
  { route: 'crons', label: 'Crons' },
  { route: 'approvals', label: 'Approbations' },
  { route: 'webhooks', label: 'Webhooks' },
]);

export const CLIENT_AUTOMATION_TABS = Object.freeze(
  AUTOMATION_TABS.filter(({ route }) => ['workflows', 'approvals'].includes(route)),
);

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

export function primaryHubsForMode(clientMode) {
  return clientMode ? CLIENT_PRIMARY_HUBS : PRIMARY_HUBS;
}

export function routeForMode(route, clientMode) {
  if (!clientMode) return route || 'chat';
  const candidate = route || 'chat';
  return ['chat', 'projects', 'automation', 'workflows', 'approvals', 'status'].includes(candidate)
    ? candidate
    : 'chat';
}

export function hubForMode(route, clientMode) {
  return hubForRoute(routeForMode(route, clientMode));
}

export function automationTabsForMode(clientMode) {
  return clientMode ? CLIENT_AUTOMATION_TABS : AUTOMATION_TABS;
}

export function automationTabForRoute(route, clientMode = false) {
  const tabs = automationTabsForMode(clientMode);
  return tabs.find((tab) => tab.route === route) || tabs[0];
}

export function capabilityTabForRoute(route) {
  return CAPABILITY_TABS.find((tab) => tab.route === route) || CAPABILITY_TABS[0];
}
