//! Color palette matching the Captain landing page design system.
//!
//! Core palette from globals.css + code syntax from constants.ts.

#![allow(dead_code)] // Full palette — some colors reserved for future screens.

use ratatui::style::{Color, Modifier, Style};

// ── Core Palette (dark mode for terminal) ───────────────────────────────────

pub const ACCENT: Color = Color::Rgb(212, 168, 83); // #D4A853
pub const ACCENT_DIM: Color = Color::Rgb(170, 134, 66); // #AA8642

// ── Legacy warm accents and Captain signature colors ────────────────────────
pub const GOLD: Color = Color::Rgb(255, 215, 0); // #FFD700
pub const AMBER: Color = Color::Rgb(255, 191, 0); // #FFBF00
pub const BRONZE: Color = Color::Rgb(205, 127, 50); // #CD7F32
pub const GOLD_DIM: Color = Color::Rgb(184, 134, 11); // #B8860B
pub const LIME: Color = Color::Rgb(191, 253, 0); // #BFFD00
pub const AQUA: Color = Color::Rgb(90, 180, 214); // #5AB4D6

pub const BG_PRIMARY: Color = Color::Rgb(15, 14, 14); // #0F0E0E — dark background
pub const BG_CARD: Color = Color::Rgb(31, 29, 28); // #1F1D1C — dark surface
pub const BG_HOVER: Color = Color::Rgb(42, 39, 37); // #2A2725 — dark hover
pub const BG_CODE: Color = Color::Rgb(24, 22, 21); // #181615 — dark code block

pub const TEXT_PRIMARY: Color = Color::Rgb(240, 239, 238); // #F0EFEE — light text on dark bg
pub const TEXT_SECONDARY: Color = Color::Rgb(168, 162, 158); // #A8A29E — muted text
pub const TEXT_TERTIARY: Color = Color::Rgb(120, 113, 108); // #78716C — dim text

pub const BORDER: Color = Color::Rgb(63, 59, 56); // #3F3B38 — dark border

// ── Semantic Colors (brighter variants for dark background contrast) ────────

pub const GREEN: Color = Color::Rgb(34, 197, 94); // #22C55E — success
pub const BLUE: Color = Color::Rgb(59, 130, 246); // #3B82F6 — info
pub const YELLOW: Color = Color::Rgb(234, 179, 8); // #EAB308 — warning
pub const RED: Color = Color::Rgb(239, 68, 68); // #EF4444 — error
pub const PURPLE: Color = Color::Rgb(168, 85, 247); // #A855F7 — decorators

// ── Backward-compat aliases ─────────────────────────────────────────────────

pub const CYAN: Color = BLUE;
pub const DIM: Color = TEXT_SECONDARY;
pub const TEXT: Color = TEXT_PRIMARY;

// ── Reusable styles ─────────────────────────────────────────────────────────

pub fn title_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn selected_style() -> Style {
    Style::default().fg(ACCENT).bg(BG_HOVER)
}

pub fn dim_style() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

pub fn input_style() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn hint_style() -> Style {
    Style::default().fg(TEXT_TERTIARY)
}

// ── Tab bar styles ──────────────────────────────────────────────────────────

pub fn tab_active() -> Style {
    Style::default()
        .fg(Color::White)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn tab_inactive() -> Style {
    Style::default().fg(TEXT_SECONDARY)
}

// ── State badge styles ──────────────────────────────────────────────────────

pub fn badge_running() -> Style {
    Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
}

pub fn badge_created() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn badge_suspended() -> Style {
    Style::default().fg(YELLOW).add_modifier(Modifier::BOLD)
}

pub fn badge_terminated() -> Style {
    Style::default().fg(TEXT_TERTIARY)
}

pub fn badge_crashed() -> Style {
    Style::default().fg(RED).add_modifier(Modifier::BOLD)
}

/// Return badge text + style for an agent state string.
pub fn state_badge(state: &str) -> (&'static str, Style) {
    let lower = state.to_lowercase();
    if lower.contains("run") {
        ("[RUN]", badge_running())
    } else if lower.contains("creat") || lower.contains("new") || lower.contains("idle") {
        ("[NEW]", badge_created())
    } else if lower.contains("sus") || lower.contains("paus") {
        ("[SUS]", badge_suspended())
    } else if lower.contains("term") || lower.contains("stop") || lower.contains("end") {
        ("[END]", badge_terminated())
    } else if lower.contains("err") || lower.contains("crash") || lower.contains("fail") {
        ("[ERR]", badge_crashed())
    } else {
        ("[---]", dim_style())
    }
}

// ── Table / channel styles ──────────────────────────────────────────────────

pub fn table_header() -> Style {
    Style::default()
        .fg(ACCENT)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

pub fn channel_ready() -> Style {
    Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
}

pub fn channel_missing() -> Style {
    Style::default().fg(YELLOW)
}

pub fn channel_off() -> Style {
    dim_style()
}

// ── Spinner ─────────────────────────────────────────────────────────────────

pub const SPINNER_FRAMES: &[&str] = &[
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

// ── Color math ──────────────────────────────────────────────────────────────

/// Linear interpolation between two RGB colors. `t` is clamped to [0.0, 1.0].
/// Non-RGB input colors return `a` unchanged.
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let lerp = |x: u8, y: u8| ((x as f32) + ((y as f32) - (x as f32)) * t).round() as u8;
            Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
        }
        _ => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_endpoints_identity() {
        let a = Color::Rgb(10, 20, 30);
        let b = Color::Rgb(100, 150, 200);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
    }

    #[test]
    fn mix_midpoint_is_average() {
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(200, 100, 50);
        assert_eq!(mix(a, b, 0.5), Color::Rgb(100, 50, 25));
    }

    #[test]
    fn mix_clamps_out_of_range() {
        let a = Color::Rgb(10, 20, 30);
        let b = Color::Rgb(100, 150, 200);
        assert_eq!(mix(a, b, -1.0), a);
        assert_eq!(mix(a, b, 2.0), b);
    }

    #[test]
    fn mix_non_rgb_returns_a() {
        let a = Color::Red;
        let b = Color::Rgb(100, 100, 100);
        assert_eq!(mix(a, b, 0.5), Color::Red);
    }
}
