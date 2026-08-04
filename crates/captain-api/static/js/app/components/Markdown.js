import { h } from '/assets/app/vendor/preact.module.js';
import { useMemo, useRef, useEffect } from '/assets/app/vendor/hooks.module.js';
import { marked } from '/assets/app/vendor/marked.esm.js';
import DOMPurify from '/assets/app/vendor/purify.es.mjs';

marked.setOptions({ gfm: true, breaks: true });

const MARKDOWN_TAGS = [
  'a', 'blockquote', 'br', 'code', 'del', 'details', 'em', 'h1', 'h2', 'h3',
  'h4', 'h5', 'h6', 'hr', 'kbd', 'li', 'ol', 'p', 'pre', 's', 'strong',
  'summary', 'table', 'tbody', 'td', 'th', 'thead', 'tr', 'ul',
];

const MARKDOWN_ATTRIBUTES = [
  'class', 'colspan', 'href', 'rel', 'rowspan', 'start', 'target', 'title',
];

const SAFE_LINK_PROTOCOLS = new Set(['http:', 'https:', 'mailto:', 'tel:']);

// Every rendered string comes from the LLM or tool output. Keep only the
// structural subset needed by Markdown and reject active/phishing HTML.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName === 'A') {
    const href = node.getAttribute('href');
    if (href) {
      try {
        const parsed = new URL(href, document.baseURI);
        if (!SAFE_LINK_PROTOCOLS.has(parsed.protocol)) node.removeAttribute('href');
      } catch {
        node.removeAttribute('href');
      }
    }
    node.setAttribute('target', '_blank');
    node.setAttribute('rel', 'noopener noreferrer');
  }
});

export function renderMarkdown(text) {
  const raw = marked.parse(text || '');
  return DOMPurify.sanitize(raw, {
    ALLOWED_TAGS: MARKDOWN_TAGS,
    ALLOWED_ATTR: MARKDOWN_ATTRIBUTES,
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
  });
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
