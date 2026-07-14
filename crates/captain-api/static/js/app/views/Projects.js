import { h } from 'preact';
import { useState, useEffect, useCallback } from 'preact/hooks';
import htm from 'htm';
import { api } from '../api.js';
import { toast } from '../store.js';
import { ProjectRuntime } from './ProjectRuntime.js';

const html = htm.bind(h);

const STATUSES = ['todo', 'doing', 'blocked', 'review', 'done', 'cancelled'];
const STATUS_LABELS = {
  todo: 'À faire', doing: 'En cours', blocked: 'Bloqué',
  review: 'En revue', done: 'Terminé', cancelled: 'Annulé',
};
const PROJECT_STATUS_LABELS = {
  planning: 'Planification', active: 'Actif', paused: 'En pause',
  done: 'Terminé', archived: 'Archivé',
};

export function Projects() {
  const [projects, setProjects] = useState(null);
  const [selectedId, setSelectedId] = useState(null);
  const [detail, setDetail] = useState(null);
  const [runtime, setRuntime] = useState(null);
  const [operatorStatus, setOperatorStatus] = useState(null);
  const [view, setView] = useState('list');
  const [showNewProject, setShowNewProject] = useState(false);
  const [showNewTask, setShowNewTask] = useState(false);

  const loadProjects = useCallback(async () => {
    try {
      const res = await api.projects();
      setProjects(res.projects || []);
    } catch (e) {
      toast(`Chargement des projets impossible : ${e.message}`, 'err');
      setProjects([]);
    }
  }, []);

  const loadDetail = useCallback(async (id) => {
    try {
      const res = await api.projectResume(id);
      setDetail(res);
    } catch (e) {
      toast(`Chargement du projet impossible : ${e.message}`, 'err');
    }
  }, []);

  const loadRuntime = useCallback(async (id) => {
    try {
      const res = await api.projectRuntime(id);
      setRuntime(res.runtime);
      setOperatorStatus(res.operator_status);
    } catch (e) {
      toast(`Chargement du runtime impossible : ${e.message}`, 'err');
    }
  }, []);

  useEffect(() => { loadProjects(); }, [loadProjects]);
  useEffect(() => {
    if (selectedId) {
      loadDetail(selectedId);
      loadRuntime(selectedId);
    }
  }, [selectedId, loadDetail, loadRuntime]);

  // Mirrors roadmap.js's syncProjectRuntimePoll: only poll while a run is
  // actually live, so idle projects don't hammer the API every 2.5s.
  useEffect(() => {
    if (!selectedId || !runtime) return;
    const live = runtime.status === 'running' || (runtime.orchestrator && runtime.orchestrator.active);
    if (!live) return;
    const t = setInterval(() => loadRuntime(selectedId), 2500);
    return () => clearInterval(t);
  }, [selectedId, runtime, loadRuntime]);

  const createProject = async (body) => {
    try {
      await api.createProject(body);
      setShowNewProject(false);
      toast('Projet créé');
      await loadProjects();
    } catch (e) {
      toast(`Création impossible : ${e.message}`, 'err');
    }
  };

  const createTask = async (body) => {
    try {
      await api.createTask(selectedId, body);
      setShowNewTask(false);
      toast('Tâche créée');
      await loadDetail(selectedId);
    } catch (e) {
      toast(`Création impossible : ${e.message}`, 'err');
    }
  };

  const changeTaskStatus = async (taskId, status) => {
    try {
      await api.updateTask(taskId, { status });
      await loadDetail(selectedId);
    } catch (e) {
      toast(`Mise à jour impossible : ${e.message}`, 'err');
    }
  };

  const deleteTask = async (taskId) => {
    try {
      await api.deleteTask(taskId);
      toast('Tâche supprimée');
      await loadDetail(selectedId);
    } catch (e) {
      toast(`Suppression impossible : ${e.message}`, 'err');
    }
  };

  if (selectedId) {
    const project = detail && detail.project;
    const tasks = (detail && detail.tasks) || [];
    return html`
      <div class="page">
        <div class="page-inner">
          <button class="ghost" onClick=${() => { setSelectedId(null); setDetail(null); setRuntime(null); setOperatorStatus(null); setView('list'); }}>← Projets</button>
          ${!detail && html`<div class="skeleton" style="height:70px;margin-top:14px"></div>`}
          ${project && html`
            <div class="project-detail-head">
              <h1 class="page-title">${project.name}</h1>
              ${project.goal && html`<p class="page-sub">${project.goal}</p>`}
            </div>
          `}
          <${ProjectRuntime} projectId=${selectedId} runtime=${runtime} operatorStatus=${operatorStatus}
            onRefresh=${() => loadRuntime(selectedId)} />
          <div class="task-toolbar">
            <div class="view-toggle">
              <button class=${view === 'list' ? 'active' : ''} onClick=${() => setView('list')}>Liste</button>
              <button class=${view === 'kanban' ? 'active' : ''} onClick=${() => setView('kanban')}>Kanban</button>
            </div>
            <span class="spacer"></span>
            <button class="primary" onClick=${() => setShowNewTask((s) => !s)}>+ Tâche</button>
          </div>
          ${showNewTask && html`<${NewTaskForm} onCreate=${createTask} onCancel=${() => setShowNewTask(false)} />`}
          ${detail && tasks.length === 0 && html`
            <div class="empty-state">
              <div class="glyph">☑</div>
              <div>Aucune tâche pour l'instant.</div>
            </div>
          `}
          ${detail && tasks.length > 0 && (view === 'list'
            ? html`<${TaskList} tasks=${tasks} onStatusChange=${changeTaskStatus} onDelete=${deleteTask} />`
            : html`<${TaskKanban} tasks=${tasks} onStatusChange=${changeTaskStatus} />`)}
        </div>
      </div>
    `;
  }

  return html`
    <div class="page">
      <div class="page-inner">
        <h1 class="page-title">Projects</h1>
        <p class="page-sub">Projets et tâches suivis par Captain.</p>
        <div class="task-toolbar">
          <span class="spacer"></span>
          <button class="primary" onClick=${() => setShowNewProject((s) => !s)}>+ Nouveau projet</button>
        </div>
        ${showNewProject && html`<${NewProjectForm} onCreate=${createProject} onCancel=${() => setShowNewProject(false)} />`}
        ${projects === null && html`
          <div class="skeleton" style="height:90px;margin-bottom:14px"></div>
          <div class="skeleton" style="height:90px"></div>
        `}
        ${projects && projects.length === 0 && html`
          <div class="empty-state">
            <div class="glyph">📁</div>
            <div>Aucun projet pour l'instant.</div>
          </div>
        `}
        ${projects && projects.length > 0 && html`
          <div class="project-grid">
            ${projects.map((p) => html`
              <div class="project-card" key=${p.id} onClick=${() => setSelectedId(p.id)}>
                <div class="project-card-head">
                  <strong>${p.name}</strong>
                  <span class="status-pill status-${p.status}">${PROJECT_STATUS_LABELS[p.status] || p.status}</span>
                </div>
                ${p.goal && html`<p class="project-card-goal">${p.goal}</p>`}
                ${p.deadline && html`<div class="project-card-meta">Échéance : ${formatDate(p.deadline)}</div>`}
              </div>
            `)}
          </div>
        `}
      </div>
    </div>
  `;
}

function TaskList({ tasks, onStatusChange, onDelete }) {
  return html`
    <div class="task-list">
      ${tasks.map((t) => html`
        <div class="task-row" key=${t.id}>
          <span class="status-pill status-${t.status}">${STATUS_LABELS[t.status] || t.status}</span>
          <span class="task-row-title">${t.title}</span>
          <span class="task-row-meta">${t.deadline ? formatDate(t.deadline) : ''}</span>
          <select value=${t.status} onChange=${(e) => onStatusChange(t.id, e.target.value)}>
            ${STATUSES.map((s) => html`<option value=${s}>${STATUS_LABELS[s]}</option>`)}
          </select>
          <button class="ghost danger" title="Supprimer" onClick=${() => onDelete(t.id)}>🗑</button>
        </div>
      `)}
    </div>
  `;
}

function TaskKanban({ tasks, onStatusChange }) {
  const [dragId, setDragId] = useState(null);

  return html`
    <div class="kanban-board">
      ${STATUSES.map((s) => {
        const col = tasks.filter((t) => t.status === s);
        return html`
          <div class="kanban-col"
            onDragOver=${(e) => e.preventDefault()}
            onDrop=${(e) => { e.preventDefault(); if (dragId) onStatusChange(dragId, s); setDragId(null); }}>
            <div class="kanban-col-head">
              <span>${STATUS_LABELS[s]}</span>
              <span class="kanban-count">${col.length}</span>
            </div>
            <div class="kanban-col-body">
              ${col.map((t) => html`
                <div class="kanban-card" key=${t.id} draggable="true"
                  onDragStart=${() => setDragId(t.id)}
                  onDragEnd=${() => setDragId(null)}>
                  <div class="kanban-card-title">${t.title}</div>
                  ${t.deadline && html`<div class="kanban-card-meta">${formatDate(t.deadline)}</div>`}
                </div>
              `)}
              ${col.length === 0 && html`<div class="kanban-empty">—</div>`}
            </div>
          </div>
        `;
      })}
    </div>
  `;
}

function NewTaskForm({ onCreate, onCancel }) {
  const [title, setTitle] = useState('');
  const [priority, setPriority] = useState('0');
  const [deadline, setDeadline] = useState('');

  const submit = (e) => {
    e.preventDefault();
    if (!title.trim()) return;
    onCreate({
      title: title.trim(),
      priority: Number(priority) || 0,
      deadline: deadline ? new Date(deadline).getTime() : null,
    });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <input type="text" placeholder="Titre de la tâche" value=${title}
        onInput=${(e) => setTitle(e.target.value)} style="flex:1" autofocus />
      <input type="number" title="Priorité" value=${priority}
        onInput=${(e) => setPriority(e.target.value)} style="width:80px" />
      <input type="date" value=${deadline} onInput=${(e) => setDeadline(e.target.value)} style="width:150px" />
      <button class="primary" type="submit">Ajouter</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}

function NewProjectForm({ onCreate, onCancel }) {
  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [slugTouched, setSlugTouched] = useState(false);
  const [goal, setGoal] = useState('');

  const submit = (e) => {
    e.preventDefault();
    const finalSlug = (slugTouched ? slug : slugify(name)).trim();
    if (!name.trim() || !finalSlug) return;
    onCreate({ name: name.trim(), slug: finalSlug, goal: goal.trim() });
  };

  return html`
    <form class="inline-form" onSubmit=${submit}>
      <input type="text" placeholder="Nom du projet" value=${name}
        onInput=${(e) => { setName(e.target.value); if (!slugTouched) setSlug(slugify(e.target.value)); }}
        style="flex:1" autofocus />
      <input type="text" placeholder="slug" value=${slug}
        onInput=${(e) => { setSlugTouched(true); setSlug(e.target.value); }} style="width:160px" />
      <input type="text" placeholder="Objectif (optionnel)" value=${goal}
        onInput=${(e) => setGoal(e.target.value)} style="flex:1" />
      <button class="primary" type="submit">Créer</button>
      <button class="ghost" type="button" onClick=${onCancel}>Annuler</button>
    </form>
  `;
}

function slugify(s) {
  return s.toLowerCase().trim().replace(/[^a-z0-9]+/g, '-').replace(/(^-|-$)/g, '');
}

function formatDate(ms) {
  try {
    return new Date(ms).toLocaleDateString();
  } catch {
    return '';
  }
}
