use super::*;

#[test]
fn image_without_path_opens_image_picker() {
    assert_eq!(
        attachment_for("/image", ""),
        Some(SlashAttachment::OpenPicker(PickerKind::Image))
    );
}

#[test]
fn file_without_path_opens_file_picker() {
    assert_eq!(
        attachment_for("/file", "   "),
        Some(SlashAttachment::OpenPicker(PickerKind::File))
    );
}

#[test]
fn paths_are_trimmed_before_attach() {
    assert_eq!(
        attachment_for("/image", "  ~/Desktop/capture.png  "),
        Some(SlashAttachment::AttachPath("~/Desktop/capture.png"))
    );
    assert_eq!(
        attachment_for("/file", "\t/tmp/report.pdf\n"),
        Some(SlashAttachment::AttachPath("/tmp/report.pdf"))
    );
}

#[test]
fn non_attachment_commands_are_ignored() {
    assert_eq!(attachment_for("/status", "image.png"), None);
}

#[test]
fn upload_status_messages_preserve_hermes_full_tui_text() {
    assert_eq!(
        upload_requires_daemon_message(),
        "Les images requièrent le mode daemon avec un agent actif."
    );
    assert_eq!(
        upload_staged_message("capture.png", "abcdef123456"),
        "📎 capture.png joint (abcdef12). Sera envoyé avec le prochain message."
    );
    assert_eq!(
        upload_missing_file_id_message(),
        "Upload OK mais réponse sans file_id."
    );
    assert_eq!(
        upload_http_error_message("413 Payload Too Large"),
        "Upload échoué: HTTP 413 Payload Too Large"
    );
    assert_eq!(
        upload_error_message("connection refused"),
        "Upload échoué: connection refused"
    );
}

#[test]
fn picker_and_inprocess_messages_preserve_hermes_text() {
    assert_eq!(
        attachments_ignored_without_daemon_message(),
        "Pièces jointes ignorées: requièrent le mode daemon."
    );
    assert_eq!(
        picker_open_error_message("permission denied"),
        "Impossible d'ouvrir l'explorateur (permission denied)"
    );
    assert_eq!(
        picker_runtime_error_message("bad selection"),
        "Explorateur: bad selection"
    );
}
