//! Native Rich control-plane cards for Telegram.

pub fn render_telegram_ask_user_prompt(question: &str, has_buttons: bool) -> String {
    let title = if has_buttons {
        "Décision requise"
    } else {
        "Question"
    };
    format!(
        "### ❓ {title}\n\n<blockquote>{}</blockquote>",
        escape_rich_text(question)
    )
}

pub fn render_telegram_ask_user_answer(question: &str, chosen: &str) -> String {
    format!(
        "### ✓ Décision enregistrée\n\n<blockquote>{}</blockquote>\n\n<b>Choix</b>\n<pre>{}</pre>",
        escape_rich_text(question),
        escape_rich_text(chosen)
    )
}

pub fn render_telegram_ask_user_expired(question: &str) -> String {
    format!(
        "### ⏱ Question expirée\n\n<blockquote>{}</blockquote>",
        escape_rich_text(question)
    )
}

pub fn render_telegram_channel_error(message: &str) -> String {
    format!(
        "### ⚠️ Captain\n\n<details open>\n<summary><b>Action interrompue</b></summary>\n\n<blockquote>{}</blockquote>\n</details>",
        escape_rich_text(message)
    )
}

fn escape_rich_text(text: &str) -> String {
    html_escape::encode_text(text.trim()).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_user_cards_distinguish_prompt_answer_and_expiry() {
        let prompt = render_telegram_ask_user_prompt("Déployer ?", true);
        assert!(prompt.starts_with("### ❓ Décision requise"));
        assert!(prompt.contains("Déployer ?"));

        let answer = render_telegram_ask_user_answer("Déployer ?", "Oui");
        assert!(answer.starts_with("### ✓ Décision enregistrée"));
        assert!(answer.contains("<pre>Oui</pre>"));

        let expired = render_telegram_ask_user_expired("Déployer ?");
        assert!(expired.starts_with("### ⏱ Question expirée"));
    }

    #[test]
    fn control_cards_escape_model_and_provider_content() {
        let hostile = "</blockquote><script>alert(1)</script>";
        for body in [
            render_telegram_ask_user_prompt(hostile, false),
            render_telegram_ask_user_answer(hostile, hostile),
            render_telegram_ask_user_expired(hostile),
            render_telegram_channel_error(hostile),
        ] {
            assert!(!body.contains("<script>"));
            assert!(body.contains("&lt;script&gt;"));
        }
    }
}
