use super::screens::file_picker::PickerKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashAttachment<'a> {
    OpenPicker(PickerKind),
    AttachPath(&'a str),
}

pub(crate) fn attachment_for<'a>(command: &str, args: &'a str) -> Option<SlashAttachment<'a>> {
    let kind = match command {
        "/image" => PickerKind::Image,
        "/file" => PickerKind::File,
        _ => return None,
    };

    let path = args.trim();
    if path.is_empty() {
        Some(SlashAttachment::OpenPicker(kind))
    } else {
        Some(SlashAttachment::AttachPath(path))
    }
}

pub(crate) fn upload_requires_daemon_message() -> &'static str {
    "Les images requièrent le mode daemon avec un agent actif."
}

pub(crate) fn upload_staged_message(filename: &str, file_id: &str) -> String {
    format!(
        "📎 {} joint ({}). Sera envoyé avec le prochain message.",
        filename,
        file_id.chars().take(8).collect::<String>()
    )
}

pub(crate) fn upload_missing_file_id_message() -> &'static str {
    "Upload OK mais réponse sans file_id."
}

pub(crate) fn upload_http_error_message(status: impl std::fmt::Display) -> String {
    format!("Upload échoué: HTTP {status}")
}

pub(crate) fn upload_error_message(error: impl std::fmt::Display) -> String {
    format!("Upload échoué: {error}")
}

pub(crate) fn attachments_ignored_without_daemon_message() -> &'static str {
    "Pièces jointes ignorées: requièrent le mode daemon."
}

pub(crate) fn picker_open_error_message(error: impl std::fmt::Display) -> String {
    format!("Impossible d'ouvrir l'explorateur ({error})")
}

pub(crate) fn picker_runtime_error_message(error: impl std::fmt::Display) -> String {
    format!("Explorateur: {error}")
}

#[cfg(test)]
#[path = "slash_attachment/tests.rs"]
mod tests;
