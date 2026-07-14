#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashScroll {
    Top,
    Bottom,
}

pub(crate) fn scroll_for(command: &str) -> Option<SlashScroll> {
    match command {
        "/top" => Some(SlashScroll::Top),
        "/bottom" => Some(SlashScroll::Bottom),
        _ => None,
    }
}

#[cfg(test)]
#[path = "slash_scroll/tests.rs"]
mod tests;
