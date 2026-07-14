//! Input cursor and wrapping calculations for the chat screen.

#[cfg(test)]
mod tests;

/// Translate an absolute byte index into `(line_index, byte_offset_in_line)`
/// over lines split on `\n`.
pub(super) fn locate_cursor(text: &str, byte_idx: usize) -> (usize, usize) {
    let clamped = byte_idx.min(text.len());
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if i >= clamped {
            break;
        }
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    (line, clamped - line_start)
}

/// Number of visual rows the input draft will occupy after newline expansion
/// and viewport wrapping. The renderer adds a prompt prefix and cursor block,
/// so the content budget is `outer_width - 4`. Returns at least 1.
pub(super) fn compute_input_visual_rows(input: &str, outer_width: u16) -> u16 {
    let prompt_and_cursor: u16 = 4;
    let avail = outer_width.saturating_sub(prompt_and_cursor).max(1) as usize;
    let logical: Vec<&str> = if input.is_empty() {
        vec![""]
    } else {
        input.split('\n').collect()
    };
    let total: usize = logical
        .iter()
        .map(|line| {
            let chars = line.chars().count();
            if chars == 0 {
                1
            } else {
                chars.div_ceil(avail).max(1)
            }
        })
        .sum();
    total.max(1) as u16
}
