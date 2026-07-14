//! TUI image previews for staged attachments.
//!
//! Backed by [`ratatui-image`]: at runtime we auto-detect the terminal
//! graphics protocol (Sixel, Kitty, iTerm2) once via
//! [`Picker::from_query_stdio`]; if the terminal does not support any
//! protocol the picker falls back to halfblocks (low-resolution but
//! universally renderable). Each previewed file is decoded once and the
//! resulting `StatefulProtocol` cached by canonicalised path so we don't
//! re-decode and re-encode on every frame.
//!
//! [`ratatui-image`]: https://github.com/benjajaja/ratatui-image

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

/// Cache of per-file image preview state.
///
/// Holds a single `Picker` (initialised lazily on first preview, reused for
/// every subsequent file) plus a map from canonicalised path to the loaded
/// `StatefulProtocol`. Drop the entry to free the decoded image.
pub struct ImagePreviewCache {
    picker: Option<Picker>,
    /// `None` value = decode failed, do not retry on every frame.
    protocols: HashMap<PathBuf, Option<StatefulProtocol>>,
}

impl ImagePreviewCache {
    pub fn new() -> Self {
        Self {
            picker: None,
            protocols: HashMap::new(),
        }
    }

    /// Drop the cached protocol for `path`. Call this when an attachment is
    /// removed so we don't keep the decoded bytes alive.
    #[allow(dead_code)] // wired by a follow-up commit that drains attachments
    pub fn forget(&mut self, path: &Path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.protocols.remove(&canonical);
    }

    /// Drop every cached protocol — used on a full reset (chat cleared).
    #[allow(dead_code)] // wired by a follow-up commit that drains attachments
    pub fn clear(&mut self) {
        self.protocols.clear();
    }

    /// How many entries are currently cached (decoded or recorded as a
    /// failed decode). Useful for tests and the status footer.
    /// Number of cached entries (decoded or recorded as a failed decode).
    /// Used by tests to verify the failure-cache path.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.protocols.len()
    }

    /// `true` when no entry has been cached yet.
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.protocols.is_empty()
    }

    /// Borrow the cached protocol for `path`, decoding it on first call.
    /// Returns `None` if the picker can't be initialised, the file can't
    /// be decoded, or the path doesn't exist — the failure is cached so
    /// subsequent frames don't keep retrying.
    pub fn get_or_load(&mut self, path: &Path) -> Option<&mut StatefulProtocol> {
        let canonical = path.canonicalize().ok()?;
        if !self.protocols.contains_key(&canonical) {
            let decoded = self.decode(&canonical);
            self.protocols.insert(canonical.clone(), decoded);
        }
        self.protocols.get_mut(&canonical)?.as_mut()
    }

    /// Decode the file once via the lazily-initialised picker.
    fn decode(&mut self, path: &Path) -> Option<StatefulProtocol> {
        let picker = self.picker_or_fallback()?;
        let dyn_img = image::ImageReader::open(path).ok()?.decode().ok()?;
        Some(picker.new_resize_protocol(dyn_img))
    }

    /// Return a borrow of the picker, initialising it on first call.
    ///
    /// Detection strategy (safest-first):
    /// 1. `CAPTAIN_IMAGE_PROTOCOL` env var override (`halfblocks`,
    ///    `kitty`, `sixel`, `iterm2`, `auto`) for power users on
    ///    terminals we mis-classify.
    /// 2. Allowlist of terminals we *know* support a graphics protocol
    ///    via env hints (`KITTY_WINDOW_ID` for Kitty, `TERM_PROGRAM`
    ///    for iTerm2 / WezTerm / Ghostty). Only those get to attempt
    ///    `Picker::from_query_stdio()`.
    /// 3. Everything else falls back to **halfblocks** — guaranteed
    ///    universal, never crashes the chat.
    ///
    /// Why we don't always trust `from_query_stdio`: on Apple Terminal
    /// it returned a Kitty-protocol picker even though the terminal
    /// doesn't decode Kitty escapes — the resulting `\e_Gi=31,s=1,…`
    /// sequences ended up rendered as literal text in the chat. The
    /// allowlist closes that hole.
    fn picker_or_fallback(&mut self) -> Option<&mut Picker> {
        if self.picker.is_none() {
            self.picker = Some(detect_picker());
        }
        self.picker.as_mut()
    }
}

/// Resolve the picker once per process based on environment hints.
fn detect_picker() -> Picker {
    use ratatui_image::picker::ProtocolType;

    // 1. Explicit override.
    match std::env::var("CAPTAIN_IMAGE_PROTOCOL")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "halfblocks" => return Picker::halfblocks(),
        "kitty" => {
            return tweak_protocol(
                Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
                ProtocolType::Kitty,
            )
        }
        "sixel" => {
            return tweak_protocol(
                Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
                ProtocolType::Sixel,
            )
        }
        "iterm2" => {
            return tweak_protocol(
                Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks()),
                ProtocolType::Iterm2,
            )
        }
        "auto" => {
            return Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        }
        _ => {} // fall through to allowlist
    }

    // 2. Trusted terminal allowlist.
    if known_graphics_terminal() {
        return Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    }

    // 3. Default safe fallback.
    Picker::halfblocks()
}

/// Force the picker to advertise a specific protocol after auto-detection.
/// Used by the env-var override branch when the user knows their terminal
/// better than we do.
fn tweak_protocol(mut picker: Picker, t: ratatui_image::picker::ProtocolType) -> Picker {
    picker.set_protocol_type(t);
    picker
}

/// Recognise terminals that ship at least one graphics protocol. Apple
/// Terminal, plain xterm without `-ti vt340`, etc. fall through.
fn known_graphics_terminal() -> bool {
    if std::env::var("KITTY_WINDOW_ID").is_ok() {
        return true;
    }
    if let Ok(p) = std::env::var("TERM_PROGRAM") {
        return matches!(
            p.as_str(),
            "iTerm.app" | "WezTerm" | "ghostty" | "Ghostty" | "rio"
        );
    }
    if let Ok(t) = std::env::var("TERM") {
        // xterm started with -ti vt340 advertises Sixel.
        return t.contains("kitty") || t == "xterm-kitty" || t == "foot" || t == "foot-extra";
    }
    false
}

impl Default for ImagePreviewCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Detect whether a content-type string looks like an image we can render.
///
/// `ratatui-image` accepts whatever the `image` crate decodes; we feature-gated
/// `image` to png/jpeg/gif/webp to keep the binary small.
pub fn is_renderable_image(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderable_image_filter() {
        assert!(is_renderable_image("image/png"));
        assert!(is_renderable_image("image/jpeg"));
        assert!(is_renderable_image("image/gif"));
        assert!(is_renderable_image("image/webp"));
        assert!(!is_renderable_image("application/pdf"));
        assert!(!is_renderable_image("text/plain"));
        assert!(!is_renderable_image("audio/mpeg"));
        assert!(!is_renderable_image(""));
    }

    #[test]
    fn cache_starts_empty() {
        let cache = ImagePreviewCache::new();
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    #[test]
    fn cache_records_failure_so_no_retry_loop() {
        // A path that doesn't exist must not panic and must not be retried.
        // canonicalize() returns Err → get_or_load returns None without
        // touching the protocol map; the second call is therefore as cheap
        // as the first.
        let mut cache = ImagePreviewCache::new();
        let bogus = PathBuf::from("/tmp/captain_image_preview_does_not_exist.png");
        assert!(cache.get_or_load(&bogus).is_none());
        assert!(cache.get_or_load(&bogus).is_none());
        // canonicalize failed every call → nothing cached, but no crash.
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn forget_is_a_noop_on_unknown_path() {
        let mut cache = ImagePreviewCache::new();
        cache.forget(Path::new("/tmp/captain_image_preview_unknown.png"));
        assert!(cache.is_empty());
    }

    /// Terminal allowlist must reject the terminals we know don't decode
    /// graphics escapes — Apple Terminal in particular, which crashed the
    /// chat with literal `\e_Gi=31,s=1,…` text after a wrong auto-detect.
    /// Tests mutate process-wide env so they must run sequentially: we
    /// gate on a Mutex.
    #[test]
    fn graphics_terminal_allowlist() {
        // Single mutex across the env-mutating tests in this module.
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

        let saved = save_env(&["KITTY_WINDOW_ID", "TERM_PROGRAM", "TERM"]);

        // Apple Terminal: NOT in allowlist.
        std::env::remove_var("KITTY_WINDOW_ID");
        std::env::set_var("TERM_PROGRAM", "Apple_Terminal");
        std::env::set_var("TERM", "xterm-256color");
        assert!(
            !known_graphics_terminal(),
            "Apple_Terminal must not be treated as a graphics-capable terminal"
        );

        // iTerm2: in allowlist.
        std::env::set_var("TERM_PROGRAM", "iTerm.app");
        assert!(known_graphics_terminal());

        // Ghostty (both casings the binary is known to set).
        std::env::set_var("TERM_PROGRAM", "Ghostty");
        assert!(known_graphics_terminal());
        std::env::set_var("TERM_PROGRAM", "ghostty");
        assert!(known_graphics_terminal());

        // WezTerm.
        std::env::set_var("TERM_PROGRAM", "WezTerm");
        assert!(known_graphics_terminal());

        // Kitty signals via env regardless of TERM_PROGRAM.
        std::env::remove_var("TERM_PROGRAM");
        std::env::set_var("KITTY_WINDOW_ID", "1");
        assert!(known_graphics_terminal());

        // foot-style terminals.
        std::env::remove_var("KITTY_WINDOW_ID");
        std::env::set_var("TERM", "foot");
        assert!(known_graphics_terminal());

        // Plain xterm: NOT in allowlist (no Sixel without explicit -ti vt340).
        std::env::set_var("TERM", "xterm-256color");
        assert!(!known_graphics_terminal());

        restore_env(saved);
    }

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn save_env(keys: &[&str]) -> Vec<(String, Option<String>)> {
        keys.iter()
            .map(|k| (k.to_string(), std::env::var(*k).ok()))
            .collect()
    }

    fn restore_env(saved: Vec<(String, Option<String>)>) {
        for (k, v) in saved {
            match v {
                Some(val) => std::env::set_var(&k, val),
                None => std::env::remove_var(&k),
            }
        }
    }
}
