/// IJ.1 — Drain any user messages that arrived in `user_input_rx`
/// since the last poll. Non-blocking: if `ask_user` (a different
/// consumer) currently holds the lock we skip this round and pick
/// the messages up on the next iteration. Returns a `Vec` rather
/// than streaming so the caller can decide how to inject them.
pub(crate) fn drain_user_interjections(
    rx: &Option<std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>>,
) -> Vec<String> {
    let mut out = Vec::new();
    let Some(rx_arc) = rx else { return out };
    let Ok(mut guard) = rx_arc.try_lock() else {
        return out;
    };
    while let Ok(msg) = guard.try_recv() {
        if !msg.trim().is_empty() {
            out.push(msg);
        }
    }
    out
}

/// IJ.1 — Wrap a user interjection in a system-style note so the LLM
/// understands this message arrived *between* tool calls and must
/// decide whether to append, re-route, or merge. Plain text — the
/// runtime injects this as a `Message::user` rather than a system
/// message because Anthropic only allows one `system` per request.
pub(crate) fn format_interjection_prompt(msg: &str) -> String {
    format!(
        "[⚠️ INTERJECTION UTILISATEUR — pendant que tu travaillais, l'utilisateur \
         a écrit : « {} » (entre deux tool calls). \
         Décide explicitement : (a) finis ce que tu fais puis adresse cette nouvelle \
         demande à la fin, (b) abandonne la trajectoire actuelle si elle devient \
         obsolète, ou (c) intègre cette demande dans ce que tu es en train de faire. \
         Cite ce message dans ta réponse pour que l'utilisateur sache que tu l'as vu.]",
        msg.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ij1_drain_returns_empty_when_rx_is_none() {
        let v = drain_user_interjections(&None);
        assert!(v.is_empty());
    }

    #[tokio::test]
    async fn ij1_drain_collects_all_pending_messages_in_order() {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        tx.send("first".into()).await.unwrap();
        tx.send("second".into()).await.unwrap();
        tx.send("third".into()).await.unwrap();

        let rx_arc = Some(std::sync::Arc::new(tokio::sync::Mutex::new(rx)));
        let drained = drain_user_interjections(&rx_arc);
        assert_eq!(drained, vec!["first", "second", "third"]);
    }

    #[tokio::test]
    async fn ij1_drain_skips_empty_or_whitespace_messages() {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        tx.send("real msg".into()).await.unwrap();
        tx.send("".into()).await.unwrap();
        tx.send("   \t\n".into()).await.unwrap();
        tx.send("another".into()).await.unwrap();

        let rx_arc = Some(std::sync::Arc::new(tokio::sync::Mutex::new(rx)));
        let drained = drain_user_interjections(&rx_arc);
        assert_eq!(drained, vec!["real msg", "another"]);
    }

    #[tokio::test]
    async fn ij1_drain_is_non_blocking_when_rx_is_empty() {
        let (_tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let rx_arc = Some(std::sync::Arc::new(tokio::sync::Mutex::new(rx)));
        let drained = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            drain_user_interjections(&rx_arc)
        })
        .await
        .expect("drain must not block when the channel is empty");
        assert!(drained.is_empty());
    }

    #[tokio::test]
    async fn ij1_drain_skips_silently_when_lock_is_held_elsewhere() {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        tx.send("buffered".into()).await.unwrap();
        let rx_arc = std::sync::Arc::new(tokio::sync::Mutex::new(rx));
        let _held = rx_arc.lock().await;
        let opt = Some(std::sync::Arc::clone(&rx_arc));

        let drained = tokio::time::timeout(std::time::Duration::from_millis(100), async {
            drain_user_interjections(&opt)
        })
        .await
        .expect("drain must not block when the mutex is locked elsewhere");
        assert!(
            drained.is_empty(),
            "drain skipped this round; the buffered message stays for next time"
        );
    }

    #[test]
    fn ij1_format_prompt_includes_user_text_and_decision_clause() {
        let prompt = format_interjection_prompt("ajoute aussi la météo");
        assert!(prompt.contains("ajoute aussi la météo"));
        assert!(prompt.contains("(a)"));
        assert!(prompt.contains("(b)"));
        assert!(prompt.contains("(c)"));
        assert!(prompt.contains("INTERJECTION"));
    }

    #[test]
    fn ij1_format_prompt_trims_user_input_to_avoid_leading_whitespace() {
        let prompt = format_interjection_prompt("  \n  surprise!  \n");
        assert!(prompt.contains("« surprise! »"), "got {prompt}");
        assert!(!prompt.contains("«   "));
    }
}
