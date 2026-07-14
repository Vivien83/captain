import { h } from 'preact';
import { useState } from 'preact/hooks';
import htm from 'htm';

const html = htm.bind(h);

// Renders an ask_user question. If item.options is a non-empty array, shows
// one button per option — clicking answers immediately. Without options,
// only the question text is shown; the Composer remains the sole way to
// respond (Chat.js detects the pending question and routes free text the
// same way as a button click — see answerAskUser there).
export function AskUserPrompt({ item, onAnswer }) {
  const [sending, setSending] = useState(false);
  const hasOptions = Array.isArray(item.options) && item.options.length > 0;

  const choose = (option) => {
    if (item.answered || sending) return;
    setSending(true);
    onAnswer(item, option);
  };

  return html`
    <div class="ask-user-prompt">
      ${hasOptions && html`
        <div class="ask-user-options">
          ${item.options.map((opt) => html`
            <button key=${opt} class=${item.answer === opt ? 'primary' : 'ghost'}
              disabled=${item.answered || sending}
              onClick=${() => choose(opt)}>${opt}</button>
          `)}
        </div>
      `}
      ${item.answered && item.answer && html`
        <div class="ask-user-answer">→ ${item.answer}</div>
      `}
    </div>
  `;
}
