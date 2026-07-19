// Captain Control — API client (session-cookie auth, same-origin only).

async function request(path, options = {}) {
  const res = await fetch(path, {
    headers: { 'content-type': 'application/json', ...(options.headers || {}) },
    credentials: 'same-origin',
    ...options,
  });
  if (res.status === 401) {
    const err = new Error('unauthorized');
    err.unauthorized = true;
    throw err;
  }
  if (!res.ok) {
    let detail = '';
    try { detail = (await res.json()).error || ''; } catch { /* non-JSON body */ }
    throw new Error(detail || `${res.status} ${res.statusText}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

export function withQuery(path, params = {}) {
  const query = new URLSearchParams();
  Object.entries(params).forEach(([key, value]) => {
    if (value !== undefined && value !== null && value !== '') query.set(key, String(value));
  });
  const encoded = query.toString();
  return encoded ? `${path}?${encoded}` : path;
}

export const api = {
  get: (path) => request(path),
  post: (path, body) => request(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) }),
  del: (path) => request(path, { method: 'DELETE' }),

  authCheck: () => request('/api/auth/check'),
  login: (username, password) => request('/api/auth/login', { method: 'POST', body: JSON.stringify({ username, password }) }),

  agents: () => request('/api/agents'),
  status: () => request('/api/status'),
  usage: () => request('/api/usage/summary'),
  budget: () => request('/api/budget'),
  modelUpdates: () => request('/api/models/updates'),
  decideModelUpdate: (body) => request('/api/models/updates/decision', { method: 'POST', body: JSON.stringify(body) }),

  agentSessions: (agentId) => request(`/api/agents/${encodeURIComponent(agentId)}/sessions`),
  createSession: (agentId) => request(`/api/agents/${encodeURIComponent(agentId)}/sessions`, { method: 'POST' }),
  switchSession: (agentId, sessionId) => request(`/api/agents/${encodeURIComponent(agentId)}/sessions/${encodeURIComponent(sessionId)}/switch`, { method: 'POST' }),
  sessionEvents: (sessionId, limit = 400) => request(`/api/sessions/${encodeURIComponent(sessionId)}/events?limit=${limit}`),
  labelSession: (sessionId, label) => request(`/api/sessions/${encodeURIComponent(sessionId)}/label`, { method: 'POST', body: JSON.stringify({ label }) }),
  deleteSession: (sessionId) => request(`/api/sessions/${encodeURIComponent(sessionId)}`, { method: 'DELETE' }),
  resetSession: (sessionId) => request(`/api/sessions/${encodeURIComponent(sessionId)}/reset`, { method: 'POST' }),

  approvals: () => request('/api/approvals'),
  approve: (id) => request(`/api/approvals/${encodeURIComponent(id)}/approve`, { method: 'POST' }),
  reject: (id) => request(`/api/approvals/${encodeURIComponent(id)}/reject`, { method: 'POST' }),
  approveSession: (id) => request(`/api/approvals/${encodeURIComponent(id)}/approve_session`, { method: 'POST' }),

  projects: () => request('/api/projects'),
  createProject: (body) => request('/api/projects', { method: 'POST', body: JSON.stringify(body) }),
  projectResume: (id) => request(`/api/projects/${encodeURIComponent(id)}/resume`),
  createTask: (projectId, body) => request(`/api/projects/${encodeURIComponent(projectId)}/tasks`, { method: 'POST', body: JSON.stringify(body) }),
  updateTask: (taskId, body) => request(`/api/project-tasks/${encodeURIComponent(taskId)}`, { method: 'PATCH', body: JSON.stringify(body) }),
  deleteTask: (taskId) => request(`/api/project-tasks/${encodeURIComponent(taskId)}`, { method: 'DELETE' }),

  projectRuntime: (id, events = 80) => request(`/api/projects/${encodeURIComponent(id)}/runtime?events=${events}`),
  startProjectRuntime: (id) => request(`/api/projects/${encodeURIComponent(id)}/runtime/start`, { method: 'POST' }),
  pauseProjectRuntime: (id) => request(`/api/projects/${encodeURIComponent(id)}/runtime/pause`, { method: 'POST' }),
  resumeProjectRuntime: (id) => request(`/api/projects/${encodeURIComponent(id)}/runtime/resume`, { method: 'POST' }),
  takeoverProjectRuntime: (id) => request(`/api/projects/${encodeURIComponent(id)}/runtime/takeover`, { method: 'POST' }),
  answerProjectAsk: (id, body) => request(`/api/projects/${encodeURIComponent(id)}/runtime/answer`, { method: 'POST', body: JSON.stringify(body) }),
  respondProjectToolRequest: (id, body) => request(`/api/projects/${encodeURIComponent(id)}/runtime/tool-request`, { method: 'POST', body: JSON.stringify(body) }),

  learningReview: () => request('/api/learning/review'),
  learningCommitted: () => request('/api/learning/committed'),
  learningMetrics: () => request('/api/learning/metrics'),
  learningDecide: (id, approve) => request(`/api/learning/review/${encodeURIComponent(id)}/decide`, { method: 'POST', body: JSON.stringify({ approve }) }),

  workflows: () => request('/api/workflows'),
  createWorkflow: (body) => request('/api/workflows', { method: 'POST', body: JSON.stringify(body) }),
  runWorkflow: (id, input) => request(`/api/workflows/${encodeURIComponent(id)}/run`, { method: 'POST', body: JSON.stringify({ input }) }),
  workflowRuns: (id) => request(`/api/workflows/${encodeURIComponent(id)}/runs`),
  deleteWorkflow: (id) => request(`/api/workflows/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  triggers: () => request('/api/triggers'),
  createTrigger: (body) => request('/api/triggers', { method: 'POST', body: JSON.stringify(body) }),
  updateTrigger: (id, body) => request(`/api/triggers/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(body) }),
  deleteTrigger: (id) => request(`/api/triggers/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  fileTriggers: () => request('/api/file-triggers'),
  createFileTrigger: (body) => request('/api/file-triggers', { method: 'POST', body: JSON.stringify(body) }),
  updateFileTrigger: (id, body) => request(`/api/file-triggers/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(body) }),
  deleteFileTrigger: (id) => request(`/api/file-triggers/${encodeURIComponent(id)}`, { method: 'DELETE' }),

  cronJobs: () => request('/api/cron/jobs'),
  createCronJob: (body) => request('/api/cron/jobs', { method: 'POST', body: JSON.stringify(body) }),
  updateCronJob: (id, body) => request(`/api/cron/jobs/${encodeURIComponent(id)}`, { method: 'PUT', body: JSON.stringify(body) }),
  deleteCronJob: (id) => request(`/api/cron/jobs/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  toggleCronJob: (id, enabled) => request(`/api/cron/jobs/${encodeURIComponent(id)}/enable`, { method: 'PUT', body: JSON.stringify({ enabled }) }),
  cronJobStatus: (id) => request(`/api/cron/jobs/${encodeURIComponent(id)}/status`),
  runCronJob: (id) => request(`/api/cron/jobs/${encodeURIComponent(id)}/run`, { method: 'POST' }),

  outboundWebhooks: () => request('/api/webhooks/outbound'),
  createWebhookEndpoint: (body) => request('/api/webhooks/outbound/endpoints', { method: 'POST', body: JSON.stringify(body) }),
  updateWebhookEndpoint: (name, body) => request(`/api/webhooks/outbound/endpoints/${encodeURIComponent(name)}`, { method: 'PUT', body: JSON.stringify(body) }),
  deleteWebhookEndpoint: (name) => request(`/api/webhooks/outbound/endpoints/${encodeURIComponent(name)}`, { method: 'DELETE' }),
  testWebhook: (body) => request('/api/webhooks/outbound/test', { method: 'POST', body: JSON.stringify(body) }),

  skills: () => request('/api/skills'),
  tools: () => request('/api/tools'),
  nativeCapabilities: (params = {}) => request(withQuery('/api/capabilities/native', params)),
  nativeCapability: (name, params = {}) => request(withQuery(
    `/api/capabilities/native/${encodeURIComponent(name)}`,
    params,
  )),
  validateNativeCapability: (body) => request('/api/capabilities/native/validate', {
    method: 'POST', body: JSON.stringify(body),
  }),
  installNativeCapability: (body) => request('/api/capabilities/native/install', {
    method: 'POST', body: JSON.stringify(body),
  }),
  decideNativeCapability: (name, body) => request(
    `/api/capabilities/native/${encodeURIComponent(name)}/decision`,
    { method: 'POST', body: JSON.stringify(body) },
  ),
  rollbackNativeCapability: (name, body) => request(
    `/api/capabilities/native/${encodeURIComponent(name)}/rollback`,
    { method: 'POST', body: JSON.stringify(body) },
  ),
  disableNativeCapability: (name, params = {}) => request(withQuery(
    `/api/capabilities/native/${encodeURIComponent(name)}`,
    params,
  ), { method: 'DELETE' }),
  nativeCapabilityRuns: (limit = 40) => request(withQuery('/api/capabilities/native/runs', { limit })),
  nativeCapabilityRun: (runId) => request(`/api/capabilities/native/runs/${encodeURIComponent(runId)}`),
  resolveNativeCapabilityRun: (runId, body) => request(
    `/api/capabilities/native/runs/${encodeURIComponent(runId)}/decision`,
    { method: 'POST', body: JSON.stringify(body) },
  ),

  healthDetail: () => request('/api/health/detail'),
  security: () => request('/api/security'),
  graphStats: () => request('/api/graph/stats'),
};

// Agent chat WebSocket. Handlers: onmessage(obj), onopen(), onclose().
export function openAgentWs(agentId, handlers) {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/api/agents/${encodeURIComponent(agentId)}/ws`);
  ws.onopen = () => handlers.onopen && handlers.onopen();
  ws.onclose = () => handlers.onclose && handlers.onclose();
  ws.onerror = () => { /* onclose follows and drives the reconnect */ };
  ws.onmessage = (ev) => {
    let obj;
    try { obj = JSON.parse(ev.data); } catch { return; }
    handlers.onmessage && handlers.onmessage(obj);
  };
  return ws;
}

// Daemon-wide realtime events (memory, agent lifecycle, tool runs).
export function openEventStream(onEvent) {
  let es = null;
  let closed = false;
  const connect = () => {
    if (closed) return;
    es = new EventSource('/api/memory/events');
    es.onmessage = (ev) => {
      try { onEvent(JSON.parse(ev.data)); } catch { /* keep-alives etc. */ }
    };
    es.onerror = () => {
      es.close();
      if (!closed) setTimeout(connect, 4000);
    };
  };
  connect();
  return { close: () => { closed = true; if (es) es.close(); } };
}
