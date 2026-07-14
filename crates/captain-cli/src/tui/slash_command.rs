pub(crate) fn split_slash_command(input: &str) -> (String, &str) {
    let trimmed = trim_command_input(input);
    let Some((command_end, _)) = trimmed.char_indices().find(|(_, c)| c.is_whitespace()) else {
        return (normalize_command_token(trimmed), "");
    };
    let command = normalize_command_token(&trimmed[..command_end]);
    let args = trim_command_input(&trimmed[command_end..]);
    (command, args)
}

pub(crate) fn canonical_slash_command(command: &str, args: &str) -> String {
    if args.is_empty() {
        command.to_string()
    } else {
        format!("{command} {args}")
    }
}

fn trim_command_input(input: &str) -> &str {
    input.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '\u{0000}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
            )
    })
}

fn normalize_command_token(input: &str) -> String {
    input
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
                )
        })
        .collect()
}

#[cfg(test)]
#[path = "slash_command/tests.rs"]
mod tests;
