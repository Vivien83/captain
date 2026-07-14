use serde_json::json;

pub(crate) fn discord_message_payload(content: &str) -> serde_json::Value {
    json!({
        "content": content,
        "allowed_mentions": {
            "parse": ["users"],
            "replied_user": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::discord_message_payload;

    #[test]
    fn payload_keeps_content() {
        let payload = discord_message_payload("hello");

        assert_eq!(payload["content"], "hello");
    }

    #[test]
    fn payload_allows_only_user_mentions() {
        let payload = discord_message_payload("@everyone <@123> <@&456>");
        let parse = payload["allowed_mentions"]["parse"]
            .as_array()
            .expect("parse list");

        assert!(parse.iter().any(|item| item == "users"));
        assert!(!parse.iter().any(|item| item == "everyone"));
        assert!(!parse.iter().any(|item| item == "roles"));
        assert_eq!(payload["allowed_mentions"]["replied_user"], false);
    }
}
