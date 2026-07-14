pub(crate) fn parse_model_switch_args(args: &str) -> (&str, Option<&'static str>) {
    let mut parts = args.split_whitespace();
    let model = parts.next().unwrap_or(args).trim();
    let mut strategy = None;
    for part in parts {
        match part {
            "--new" | "--new-session" => strategy = Some("new_session"),
            "--compact" | "--compact-session" => strategy = Some("compact_session"),
            _ => {}
        }
    }
    (model, strategy)
}

pub(crate) fn path_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
#[path = "command_args/tests.rs"]
mod tests;
