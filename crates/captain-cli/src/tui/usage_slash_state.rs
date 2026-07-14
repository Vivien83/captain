use crate::i18n::Lang;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UsageSlashSnapshot {
    pub(crate) session_input_tokens: u64,
    pub(crate) session_output_tokens: u64,
    pub(crate) session_cached_input_tokens: u64,
    pub(crate) session_cache_creation_tokens: u64,
    pub(crate) last_tokens: Option<(u64, u64)>,
    pub(crate) last_cached_input_tokens: u64,
    pub(crate) session_cost_usd: f64,
    pub(crate) last_cost_usd: Option<f64>,
    pub(crate) message_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UsageSlashSurface {
    FullTui,
    StandaloneChat,
}

pub(crate) fn token_usage_message(
    snapshot: UsageSlashSnapshot,
    surface: UsageSlashSurface,
    lang: Lang,
) -> String {
    let total = snapshot.session_input_tokens + snapshot.session_output_tokens;
    if total == 0 {
        return match (surface, lang) {
            (UsageSlashSurface::StandaloneChat, Lang::En) => {
                "No tokens consumed in this session yet.".to_string()
            }
            _ => "Aucun token consommé pour cette session encore.".to_string(),
        };
    }

    let last = last_tokens_line(snapshot, lang, surface);
    let cache = cache_line(snapshot, surface);

    match surface {
        UsageSlashSurface::FullTui => {
            let turns = snapshot.message_count.max(1) as u64;
            format!(
                "Session : {}↑ in / {}↓ out  (total {})\nMoyenne par tour : ~{} in / ~{} out{cache}{last}",
                snapshot.session_input_tokens,
                snapshot.session_output_tokens,
                total,
                snapshot.session_input_tokens / turns,
                snapshot.session_output_tokens / turns,
            )
        }
        UsageSlashSurface::StandaloneChat => match lang {
            Lang::Fr => format!(
                "Session : {}↑ in / {}↓ out (total {total}){cache}{last}",
                snapshot.session_input_tokens, snapshot.session_output_tokens
            ),
            Lang::En => format!(
                "Session: {}↑ in / {}↓ out (total {total}){cache}{last}",
                snapshot.session_input_tokens, snapshot.session_output_tokens
            ),
        },
    }
}

pub(crate) fn cost_usage_message(
    snapshot: UsageSlashSnapshot,
    surface: UsageSlashSurface,
    lang: Lang,
) -> String {
    let cost = snapshot.session_cost_usd;
    if cost <= 0.0 {
        return match (surface, lang) {
            (UsageSlashSurface::StandaloneChat, Lang::En) => "Session cost: $0.0000.".to_string(),
            _ => "Coût session : $0.0000 (aucun usage facturé pour l'instant).".to_string(),
        };
    }

    match (surface, lang) {
        (UsageSlashSurface::StandaloneChat, Lang::En) => format!(
            "Session cost: ${cost:.4}\nLast turn: ${:.4}",
            snapshot.last_cost_usd.unwrap_or(0.0)
        ),
        _ => format!(
            "Coût session : ${cost:.4}\nDernier tour : ${:.4}",
            snapshot.last_cost_usd.unwrap_or(0.0)
        ),
    }
}

fn last_tokens_line(
    snapshot: UsageSlashSnapshot,
    lang: Lang,
    surface: UsageSlashSurface,
) -> String {
    let Some((input, output)) = snapshot.last_tokens else {
        return String::new();
    };
    let cached = snapshot.last_cached_input_tokens;
    if cached > 0 {
        return match (surface, lang) {
            (UsageSlashSurface::StandaloneChat, Lang::En) => {
                format!("\nLast turn: {input}↑ {output}↓ including {cached} cached")
            }
            _ => format!("\nDernier tour : {input}↑ {output}↓ dont {cached} cached"),
        };
    }
    match (surface, lang) {
        (UsageSlashSurface::StandaloneChat, Lang::En) => {
            format!("\nLast turn: {input}↑ {output}↓")
        }
        _ => format!("\nDernier tour : {input}↑ {output}↓"),
    }
}

fn cache_line(snapshot: UsageSlashSnapshot, surface: UsageSlashSurface) -> String {
    if snapshot.session_cached_input_tokens == 0 {
        return String::new();
    }
    let separator = match surface {
        UsageSlashSurface::FullTui => " : ",
        UsageSlashSurface::StandaloneChat => ": ",
    };
    format!(
        "\nCache{separator}{} input cached / {} creation",
        snapshot.session_cached_input_tokens, snapshot.session_cache_creation_tokens
    )
}

#[cfg(test)]
#[path = "usage_slash_state/tests.rs"]
mod tests;
