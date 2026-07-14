import { h, render } from 'preact';
import { useState, useEffect } from 'preact/hooks';
import htm from 'htm';
import { api, openEventStream } from './api.js';
import { getState, setState, subscribe, toast } from './store.js';
import { Login } from './components/Login.js';
import { Shell } from './components/Shell.js';
import { Chat } from './views/Chat.js';
import { Projects } from './views/Projects.js';
import { Learning } from './views/Learning.js';
import { Automation } from './views/Automation.js';
import { Capabilities } from './views/Capabilities.js';
import { Status } from './views/Status.js';
import { hubForRoute } from './control_contract.mjs';

const html = htm.bind(h);

const VIEWS = {
  chat: Chat,
  projects: Projects,
  automation: Automation,
  learning: Learning,
  capabilities: Capabilities,
  status: Status,
};

function useRoute() {
  const parse = () => (location.hash.replace(/^#\//, '') || 'chat').split('?')[0];
  const [route, setRoute] = useState(parse());
  useEffect(() => {
    const on = () => setRoute(parse());
    window.addEventListener('hashchange', on);
    return () => window.removeEventListener('hashchange', on);
  }, []);
  return route;
}

function App() {
  const [authed, setAuthed] = useState(getState().authed);
  const route = useRoute();

  useEffect(() => subscribe((s) => setAuthed(s.authed)), []);

  // auth_check always answers HTTP 200 — the actual state is the JSON body's
  // `mode`/`authenticated` fields, not the request's success. Mirrors
  // terminal.js's checkAccess() so both surfaces agree on what "logged in"
  // means (e.g. mode:"none"/"apikey" still requires configuring web login,
  // it does not mean "open access").
  useEffect(() => {
    api.authCheck()
      .then((info) => {
        if (info && info.mode === 'session' && info.authenticated) {
          setState({ authed: true });
        } else {
          setState({ authed: false, authMode: (info && info.mode) || 'unknown' });
        }
      })
      .catch(() => setState({ authed: false, authMode: 'unknown' }));
  }, []);

  // Daemon-wide realtime stream: background activity + approval nudges.
  useEffect(() => {
    if (authed !== true) return;
    const stream = openEventStream((ev) => {
      const s = getState();
      if (ev.type === 'agent_lifecycle') {
        const key = `agent:${ev.agent_id}`;
        if (ev.kind === 'spawned') {
          setState({ backgroundActivity: [...s.backgroundActivity, { key, label: ev.name || 'agent' }] });
        } else {
          setState({ backgroundActivity: s.backgroundActivity.filter((b) => b.key !== key) });
          if (ev.kind === 'terminated' || ev.kind === 'crashed') {
            toast(`Sous-agent ${ev.name || ev.agent_id} : ${ev.kind === 'crashed' ? 'planté' : 'terminé'}`,
              ev.kind === 'crashed' ? 'err' : 'ok');
          }
        }
      } else if (ev.type === 'tool_run_status') {
        const key = `run:${ev.run_id}`;
        if (ev.status === 'running') {
          setState({ backgroundActivity: [...s.backgroundActivity, { key, label: ev.tool_name }] });
        } else {
          setState({ backgroundActivity: s.backgroundActivity.filter((b) => b.key !== key) });
        }
      }
    });
    return () => stream.close();
  }, [authed]);

  if (authed === null) return html`<div></div>`;
  if (authed === false) return html`<${Login} mode=${getState().authMode} />`;

  const View = VIEWS[hubForRoute(route)] || Chat;
  return html`
    <${Shell} route=${route}>
      <${View} route=${route} />
    <//>
  `;
}

render(h(App, {}), document.getElementById('app'));
