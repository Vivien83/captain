use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportSurface {
    FullTui,
    StandaloneChat,
}

pub(crate) fn export_success_message(lang: crate::i18n::Lang, path: &Path) -> String {
    match lang {
        crate::i18n::Lang::Fr => format!("Session exportée : {}", path.display()),
        crate::i18n::Lang::En => format!("Session exported: {}", path.display()),
    }
}

pub(crate) fn export_failed_message(
    lang: crate::i18n::Lang,
    surface: ExportSurface,
    err: impl fmt::Display,
) -> String {
    match (lang, surface) {
        (crate::i18n::Lang::Fr, ExportSurface::FullTui) => format!("Échec export: {err}"),
        (crate::i18n::Lang::Fr, ExportSurface::StandaloneChat) => {
            format!("Échec export : {err}")
        }
        (crate::i18n::Lang::En, _) => format!("Export failed: {err}"),
    }
}

#[cfg(test)]
#[path = "slash_export/tests.rs"]
mod tests;
