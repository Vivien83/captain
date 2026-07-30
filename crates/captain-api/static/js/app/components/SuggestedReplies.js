import { h } from 'preact';
import htm from 'htm';

const html = htm.bind(h);

export function SuggestedReplies({ item, onChoose }) {
  const options = Array.isArray(item.suggestedReplies) ? item.suggestedReplies : [];
  if (!item.suggestionsActive || options.length === 0) return null;

  return html`
    <div class="suggested-replies">
      ${options.map((option, index) => html`
        <button key=${`${index}:${option}`} class="ghost" onClick=${() => onChoose(item, option)}>
          ${option}
        </button>
      `)}
    </div>
  `;
}
