use super::*;
use std::path::PathBuf;

fn attachment(content_type: &str, local_path: Option<&str>) -> PendingAttachment {
    PendingAttachment {
        file_id: "file-1".to_string(),
        filename: "sample.png".to_string(),
        content_type: content_type.to_string(),
        local_path: local_path.map(PathBuf::from),
    }
}

#[test]
fn preview_rows_zero_without_attachments() {
    let state = ChatState::new();

    assert_eq!(staged_image_preview_rows(&state), 0);
}

#[test]
fn preview_rows_zero_for_non_image_attachment() {
    let mut state = ChatState::new();
    state
        .pending_attachments
        .push(attachment("application/pdf", Some("/tmp/sample.pdf")));

    assert_eq!(staged_image_preview_rows(&state), 0);
}

#[test]
fn preview_rows_zero_for_pathless_image() {
    let mut state = ChatState::new();
    state
        .pending_attachments
        .push(attachment("image/png", None));

    assert_eq!(staged_image_preview_rows(&state), 0);
}

#[test]
fn preview_rows_reserves_strip_for_local_image() {
    let mut state = ChatState::new();
    state
        .pending_attachments
        .push(attachment("image/png", Some("/tmp/sample.png")));

    assert_eq!(staged_image_preview_rows(&state), 8);
}
