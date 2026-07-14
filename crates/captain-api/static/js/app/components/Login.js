import { h } from 'preact';
import { useState } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { setState } from '../store.js';

const html = htm.bind(h);

// `mode` mirrors terminal.js's checkAccess(): "session" is the only mode
// with an actual login form here; "apikey"/"none"/"unknown" mean web login
// itself isn't configured yet, so there is nothing to submit against.
export function Login({ mode }) {
  if (mode && mode !== 'session') {
    return html`
      <div class="login-screen">
        <div class="login-card">
          <img src="/assets/logo.png?rev=wordmark-2" alt="Captain" />
          <h1>Captain</h1>
          <div class="login-error" style="color:var(--text-1)">
            ${mode === 'apikey'
              ? "L'authentification web n'est pas configurée pour une connexion par session. Configure un accès web (captain setup) ou utilise une clé API."
              : "L'authentification web n'est pas configurée. Lance `captain setup` ou demande à Captain de créer des identifiants web."}
          </div>
        </div>
      </div>
    `;
  }

  return html`<${SessionLoginForm} />`;
}

function SessionLoginForm() {
  const [username, setUsername] = useState('admin');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const submit = async (e) => {
    e.preventDefault();
    setBusy(true);
    setError('');
    try {
      await api.login(username, password);
      setState({ authed: true });
    } catch {
      setError('Identifiants refusés');
    } finally {
      setBusy(false);
    }
  };

  return html`
    <div class="login-screen">
      <div class="login-card">
        <img src="/assets/logo.png?rev=wordmark-2" alt="Captain" />
        <h1>Captain</h1>
        <form onSubmit=${submit}>
          <input type="text" placeholder="Utilisateur" value=${username}
            onInput=${(e) => setUsername(e.target.value)} autocomplete="username" />
          <input type="password" placeholder="Mot de passe" value=${password}
            onInput=${(e) => setPassword(e.target.value)} autocomplete="current-password" autofocus />
          <button class="primary" type="submit" disabled=${busy}>
            ${busy ? 'Connexion…' : 'Se connecter'}
          </button>
        </form>
        <div class="login-error">${error}</div>
      </div>
    </div>
  `;
}
