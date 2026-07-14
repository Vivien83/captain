use super::*;
use std::path::Path;

#[test]
fn success_message_preserves_full_tui_french_and_standalone_english() {
    let path = Path::new("/tmp/captain/session.md");

    assert_eq!(
        export_success_message(crate::i18n::Lang::Fr, path),
        "Session exportée : /tmp/captain/session.md"
    );
    assert_eq!(
        export_success_message(crate::i18n::Lang::En, path),
        "Session exported: /tmp/captain/session.md"
    );
}

#[test]
fn failure_message_preserves_hermes_full_tui_spacing() {
    assert_eq!(
        export_failed_message(crate::i18n::Lang::Fr, ExportSurface::FullTui, "disk full"),
        "Échec export: disk full"
    );
}

#[test]
fn failure_message_preserves_standalone_localized_spacing() {
    assert_eq!(
        export_failed_message(
            crate::i18n::Lang::Fr,
            ExportSurface::StandaloneChat,
            "disk full"
        ),
        "Échec export : disk full"
    );
    assert_eq!(
        export_failed_message(
            crate::i18n::Lang::En,
            ExportSurface::StandaloneChat,
            "disk full"
        ),
        "Export failed: disk full"
    );
}
