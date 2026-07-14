//! File picker overlay used by `/image` and `/file` when the user hits
//! Enter without supplying a path.
//!
//! Backed by [`ratatui-explorer`] so we don't reinvent directory walking,
//! filtering and key bindings. The overlay opens centred over the chat,
//! defaults to the user's home directory, and returns the selected path
//! when the user presses Enter on a regular file. Esc closes without
//! selecting.
//!
//! [`ratatui-explorer`]: https://github.com/tatounee/ratatui-explorer

use std::path::PathBuf;

use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear};
use ratatui::Frame;
use ratatui_explorer::{FileExplorer, Theme};

use crate::tui::theme;

/// What the picker is collecting — gates the filter and the post-select
/// action so `/image` only accepts renderable images while `/file` is
/// permissive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    /// `/image` — only PNG/JPEG/GIF/WEBP files are accepted on Enter.
    Image,
    /// `/file` — any file accepted (subject to upload allowlist later).
    File,
}

/// The overlay's per-instance state. Wrap in `Option` on the App so a
/// `None` value means "no picker open".
pub struct FilePickerState {
    pub kind: PickerKind,
    pub explorer: FileExplorer,
    /// Last error surfaced under the explorer, e.g. "format non supporté"
    /// when the user hits Enter on a `.rar` while the picker is in Image
    /// mode. Cleared on the next navigation event.
    pub last_error: Option<String>,
}

impl FilePickerState {
    /// Open a fresh picker rooted at the user's home directory (or the
    /// current working directory if `$HOME` is not resolvable).
    pub fn open(kind: PickerKind) -> std::io::Result<Self> {
        let theme = picker_theme(kind);
        let mut explorer = FileExplorer::with_theme(theme)?;
        if let Some(home) = dirs::home_dir() {
            // Best-effort: if `$HOME` is missing or unreadable just stay
            // on the explorer's default (cwd).
            let _ = explorer.set_cwd(home);
        }
        Ok(Self {
            kind,
            explorer,
            last_error: None,
        })
    }

    /// Forward a terminal event to the picker. Returns `Some(path)` when
    /// the user has just confirmed a regular file, `None` to keep the
    /// picker open. The caller should close the overlay on `Some` and
    /// also on Esc (handled by the caller, not here).
    pub fn handle(&mut self, event: &Event) -> std::io::Result<Option<PathBuf>> {
        // Enter on a regular file = pick. Enter on a directory falls
        // through to the explorer so it descends.
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Press && key.code == KeyCode::Enter {
                let current = self.explorer.current();
                if current.is_file() {
                    if !self.is_acceptable(current.path()) {
                        self.last_error = Some(format!(
                            "Format non supporté pour /{}: {}",
                            self.kind.command_name(),
                            current
                                .path()
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("?")
                        ));
                        return Ok(None);
                    }
                    return Ok(Some(current.path().to_path_buf()));
                }
            }
        }
        // Any other event clears the inline error so the user gets fresh
        // feedback on their next navigation.
        self.last_error = None;
        self.explorer.handle(event)?;
        Ok(None)
    }

    fn is_acceptable(&self, path: &std::path::Path) -> bool {
        match self.kind {
            PickerKind::File => true,
            PickerKind::Image => path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    matches!(
                        e.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "webp"
                    )
                })
                .unwrap_or(false),
        }
    }
}

impl PickerKind {
    pub fn command_name(self) -> &'static str {
        match self {
            PickerKind::Image => "image",
            PickerKind::File => "file",
        }
    }
}

/// Build a Captain-themed picker chrome — gold border, dimmed dirs,
/// highlight on the current row.
fn picker_theme(kind: PickerKind) -> Theme {
    let title = match kind {
        PickerKind::Image => "  Choisir une image (Enter sélectionne, Esc annule)",
        PickerKind::File => "  Choisir un fichier (Enter sélectionne, Esc annule)",
    };
    Theme::default()
        .with_block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::ACCENT))
                .title(title),
        )
        .with_highlight_dir_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .with_highlight_item_style(
            Style::default()
                .fg(Color::White)
                .bg(theme::BG_HOVER)
                .add_modifier(Modifier::BOLD),
        )
        .with_dir_style(Style::default().fg(theme::ACCENT))
        .with_highlight_symbol("▶ ")
}

/// Render the picker as a centred 70×60% overlay. Caller is responsible
/// for clearing the underlying area first via `Clear` — we do it here so
/// the chat shows through neither the title nor the file list.
pub fn draw(f: &mut Frame, area: Rect, state: &FilePickerState) {
    let popup = centred_rect(area, 70, 60);
    f.render_widget(Clear, popup);
    f.render_widget(&state.explorer.widget(), popup);
    if let Some(ref err) = state.last_error {
        // Show inline at the bottom of the popup so the user sees why
        // the last Enter didn't pick the file.
        let bar_area = Rect::new(
            popup.x + 1,
            popup.y + popup.height - 2,
            popup.width.saturating_sub(2),
            1,
        );
        let bar = ratatui::widgets::Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        f.render_widget(bar, bar_area);
    }
}

/// Compute a popup rectangle centred in `parent` at `pct_w` × `pct_h`.
fn centred_rect(parent: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = parent.width.saturating_mul(pct_w) / 100;
    let h = parent.height.saturating_mul(pct_h) / 100;
    let x = parent.x + (parent.width.saturating_sub(w)) / 2;
    let y = parent.y + (parent.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_picker_filters_extensions() {
        let s = FilePickerState::open(PickerKind::Image).unwrap();
        assert!(s.is_acceptable(std::path::Path::new("foo.png")));
        assert!(s.is_acceptable(std::path::Path::new("FOO.JPG")));
        assert!(s.is_acceptable(std::path::Path::new("a/b/c.webp")));
        assert!(!s.is_acceptable(std::path::Path::new("doc.pdf")));
        assert!(!s.is_acceptable(std::path::Path::new("note.txt")));
        assert!(!s.is_acceptable(std::path::Path::new("noext")));
    }

    #[test]
    fn file_picker_accepts_any_file() {
        let s = FilePickerState::open(PickerKind::File).unwrap();
        assert!(s.is_acceptable(std::path::Path::new("foo.png")));
        assert!(s.is_acceptable(std::path::Path::new("doc.pdf")));
        assert!(s.is_acceptable(std::path::Path::new("note.txt")));
        assert!(s.is_acceptable(std::path::Path::new("noext")));
    }

    #[test]
    fn command_name_round_trip() {
        assert_eq!(PickerKind::Image.command_name(), "image");
        assert_eq!(PickerKind::File.command_name(), "file");
    }
}
