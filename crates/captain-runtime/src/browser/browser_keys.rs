pub(super) struct BrowserKeySpec {
    pub(super) key: String,
    pub(super) code: Option<String>,
    pub(super) key_code: Option<u32>,
    pub(super) text: Option<String>,
    pub(super) insert_text: Option<String>,
    pub(super) modifiers: u32,
}

pub(super) fn browser_key_spec(input: &str) -> BrowserKeySpec {
    let mut modifiers = 0;
    let mut parts = input.split('+').map(str::trim).filter(|s| !s.is_empty());
    let mut key = "";
    for part in parts.by_ref() {
        match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => modifiers |= 1,
            "ctrl" | "control" => modifiers |= 2,
            "meta" | "cmd" | "command" | "super" => modifiers |= 4,
            "shift" => modifiers |= 8,
            _ => {
                key = part;
                for tail in parts {
                    key = tail;
                }
                break;
            }
        }
    }
    if key.is_empty() {
        key = input.trim();
    }

    let lower = key.to_ascii_lowercase();
    let named = match lower.as_str() {
        "enter" | "return" => Some(("Enter", "Enter", 13)),
        "tab" => Some(("Tab", "Tab", 9)),
        "escape" | "esc" => Some(("Escape", "Escape", 27)),
        "backspace" => Some(("Backspace", "Backspace", 8)),
        "delete" | "del" => Some(("Delete", "Delete", 46)),
        "arrowup" | "up" => Some(("ArrowUp", "ArrowUp", 38)),
        "arrowdown" | "down" => Some(("ArrowDown", "ArrowDown", 40)),
        "arrowleft" | "left" => Some(("ArrowLeft", "ArrowLeft", 37)),
        "arrowright" | "right" => Some(("ArrowRight", "ArrowRight", 39)),
        "home" => Some(("Home", "Home", 36)),
        "end" => Some(("End", "End", 35)),
        "pageup" => Some(("PageUp", "PageUp", 33)),
        "pagedown" => Some(("PageDown", "PageDown", 34)),
        "space" => Some((" ", "Space", 32)),
        _ => None,
    };

    if let Some((key, code, key_code)) = named {
        return BrowserKeySpec {
            key: key.to_string(),
            code: Some(code.to_string()),
            key_code: Some(key_code),
            text: None,
            insert_text: None,
            modifiers,
        };
    }

    let mut chars = key.chars();
    let first = chars.next();
    let single_char = first.is_some() && chars.next().is_none();
    if single_char {
        let ch = first.unwrap_or_default();
        let key_string = ch.to_string();
        let code = ascii_key_code(ch).map(|c| {
            if ch.is_ascii_alphabetic() {
                let upper = ch.to_ascii_uppercase();
                format!("Key{upper}")
            } else if ch.is_ascii_digit() {
                format!("Digit{ch}")
            } else {
                c.to_string()
            }
        });
        return BrowserKeySpec {
            key: key_string.clone(),
            code,
            key_code: Some(ch.to_ascii_uppercase() as u32),
            text: if modifiers == 0 {
                Some(key_string)
            } else {
                None
            },
            insert_text: None,
            modifiers,
        };
    }

    BrowserKeySpec {
        key: key.to_string(),
        code: None,
        key_code: None,
        text: None,
        insert_text: if modifiers == 0 {
            Some(key.to_string())
        } else {
            None
        },
        modifiers,
    }
}

fn ascii_key_code(ch: char) -> Option<&'static str> {
    match ch {
        '0'..='9' | 'a'..='z' | 'A'..='Z' => Some(""),
        '-' => Some("Minus"),
        '=' => Some("Equal"),
        '[' => Some("BracketLeft"),
        ']' => Some("BracketRight"),
        '\\' => Some("Backslash"),
        ';' => Some("Semicolon"),
        '\'' => Some("Quote"),
        ',' => Some("Comma"),
        '.' => Some("Period"),
        '/' => Some("Slash"),
        '`' => Some("Backquote"),
        _ => None,
    }
}
