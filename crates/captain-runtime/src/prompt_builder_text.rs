/// Cap a string to `max_chars`, appending "..." if truncated.
pub(super) fn cap_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

/// Capitalize the first letter of a string.
pub(super) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
