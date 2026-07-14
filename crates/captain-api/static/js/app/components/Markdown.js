import { h } from 'preact';
import { useMemo, useRef, useEffect } from 'preact/hooks';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ gfm: true, breaks: true });

// Every rendered string comes from the LLM or tool output — treat all of it
// as attacker-controlled. DOMPurify strips scripts/handlers; we additionally
// force links to open in a new tab without opener access.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') {
    node.setAttribute('target', '_blank');
    node.setAttribute('rel', 'noopener noreferrer');
  }
});

export function renderMarkdown(text) {
  const raw = marked.parse(text || '');
  return DOMPurify.sanitize(raw, { USE_PROFILES: { html: true } });
}

export function Markdown({ text }) {
  const html = useMemo(() => renderMarkdown(text), [text]);
  const ref = useRef(null);

  // Copy buttons on code blocks, attached after each render.
  useEffect(() => {
    if (!ref.current) return;
    ref.current.querySelectorAll('pre').forEach((pre) => {
      if (pre.querySelector('.copy-btn')) return;
      const btn = document.createElement('button');
      btn.className = 'copy-btn';
      btn.textContent = 'copier';
      btn.addEventListener('click', () => {
        const code = pre.querySelector('code');
        navigator.clipboard.writeText(code ? code.innerText : pre.innerText);
        btn.textContent = 'copié ✓';
        setTimeout(() => { btn.textContent = 'copier'; }, 1500);
      });
      pre.appendChild(btn);
    });
  }, [html]);

  return h('div', { class: 'md', ref, dangerouslySetInnerHTML: { __html: html } });
}
