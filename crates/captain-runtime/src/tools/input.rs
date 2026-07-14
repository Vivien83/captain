//! Shared JSON input extraction helpers.

pub(crate) fn collect_string_list(input: &serde_json::Value, key: &str) -> Option<Vec<String>> {
    input.get(key).and_then(|value| {
        if let Some(single) = value.as_str() {
            Some(vec![single.to_string()])
        } else {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
        }
    })
}

pub(crate) fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
