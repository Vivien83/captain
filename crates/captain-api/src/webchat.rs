//! Embedded browser tools served as static HTML.
//!
//! Captain intentionally exposes focused browser tools: the xterm.js terminal
//! and a small authenticated config editor. The old browser SPA was removed so
//! VPS/browser usage stays focused on operational surfaces.

use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

/// Compile-time ETag based on the crate version.
const ETAG: &str = concat!("\"captain-", env!("CARGO_PKG_VERSION"), "\"");

/// GET /logo.svg — Legacy no-op endpoint kept so stale clients do not show
/// cached legacy branding.
pub async fn logo_svg() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate")],
        StatusCode::NO_CONTENT,
    )
}

/// GET /favicon.ico — Serve the embedded Captain emblem for legacy clients.
pub async fn favicon_ico() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LOGO_PNG,
    )
}

/// Embedded PWA manifest for installable web app support.
const MANIFEST_JSON: &str = include_str!("../static/manifest.json");

/// Embedded service worker for PWA support.
const SW_JS: &str = include_str!("../static/sw.js");

/// Shared Captain emblem embedded in the release binary.
const LOGO_PNG: &[u8] = include_bytes!("../../../assets/logo.png");

/// GET /manifest.json — Serve the PWA web app manifest.
pub async fn manifest_json() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        MANIFEST_JSON,
    )
}

/// GET /sw.js — Serve the PWA service worker.
pub async fn sw_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SW_JS,
    )
}

/// GET / — Captain Control, the rich Preact app (chat, sessions, approvals).
/// Assembled like the other pages: everything embedded in the binary, no CDN.
pub async fn app_page() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::ETAG, ETAG),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        APP_HTML,
    )
}

const APP_HTML: &str = concat!(
    "<!DOCTYPE html>\n<html lang=\"fr\">\n<head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<title>Captain</title>\n",
    "<link rel=\"manifest\" href=\"/manifest.json\">\n",
    "<link rel=\"icon\" type=\"image/png\" sizes=\"1254x1254\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<link rel=\"apple-touch-icon\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<style>\n",
    include_str!("../static/css/theme.css"),
    "\n",
    include_str!("../static/css/app.css"),
    "\n</style>\n",
    "</head>\n<body data-theme=\"dark\">\n",
    include_str!("../static/app_body.html"),
    "</body></html>"
);

/// GET /assets/logo.png — the Captain crown, embedded from the repo assets.
pub async fn logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        LOGO_PNG,
    )
}

/// GET /assets/app/{*path} — ES modules for the Control app. Static match
/// over embedded files only: no filesystem access, no traversal surface.
pub async fn app_asset(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    match embedded_app_asset(&path) {
        Some(content) => (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8",
                ),
                (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
            ],
            content,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn embedded_app_asset(path: &str) -> Option<&'static str> {
    match path {
        "main.js" => Some(include_str!("../static/js/app/main.js")),
        "api.js" => Some(include_str!("../static/js/app/api.js")),
        "store.js" => Some(include_str!("../static/js/app/store.js")),
        "control_contract.mjs" => Some(include_str!("../static/js/app/control_contract.mjs")),
        "status_model.mjs" => Some(include_str!("../static/js/app/status_model.mjs")),
        "provider_quota_model.mjs" => {
            Some(include_str!("../static/js/app/provider_quota_model.mjs"))
        }
        "components/Shell.js" => Some(include_str!("../static/js/app/components/Shell.js")),
        "components/Login.js" => Some(include_str!("../static/js/app/components/Login.js")),
        "components/Markdown.js" => Some(include_str!("../static/js/app/components/Markdown.js")),
        "components/ToolCard.js" => Some(include_str!("../static/js/app/components/ToolCard.js")),
        "components/AskUserPrompt.js" => {
            Some(include_str!("../static/js/app/components/AskUserPrompt.js"))
        }
        "views/Chat.js" => Some(include_str!("../static/js/app/views/Chat.js")),
        "views/Approvals.js" => Some(include_str!("../static/js/app/views/Approvals.js")),
        "views/Projects.js" => Some(include_str!("../static/js/app/views/Projects.js")),
        "views/ProjectRuntime.js" => Some(include_str!("../static/js/app/views/ProjectRuntime.js")),
        "views/Learning.js" => Some(include_str!("../static/js/app/views/Learning.js")),
        "views/Triggers.js" => Some(include_str!("../static/js/app/views/Triggers.js")),
        "views/Crons.js" => Some(include_str!("../static/js/app/views/Crons.js")),
        "views/Webhooks.js" => Some(include_str!("../static/js/app/views/Webhooks.js")),
        "views/Workflows.js" => Some(include_str!("../static/js/app/views/Workflows.js")),
        "views/Automation.js" => Some(include_str!("../static/js/app/views/Automation.js")),
        "views/Capabilities.js" => Some(include_str!("../static/js/app/views/Capabilities.js")),
        "views/NativeCapabilities.js" => {
            Some(include_str!("../static/js/app/views/NativeCapabilities.js"))
        }
        "views/Status.js" => Some(include_str!("../static/js/app/views/Status.js")),
        "vendor/preact.module.js" => Some(include_str!("../static/vendor/preact/preact.module.js")),
        "vendor/hooks.module.js" => Some(include_str!("../static/vendor/preact/hooks.module.js")),
        "vendor/htm.module.js" => Some(include_str!("../static/vendor/preact/htm.module.js")),
        "vendor/marked.esm.js" => Some(include_str!("../static/vendor/marked/marked.esm.js")),
        "vendor/purify.es.mjs" => Some(include_str!("../static/vendor/dompurify/purify.es.mjs")),
        _ => None,
    }
}

#[cfg(test)]
mod control_app_asset_tests {
    use super::{
        embedded_app_asset, favicon_ico, APP_HTML, CONFIG_HTML, LOGO_PNG, MANIFEST_JSON,
        TERMINAL_HTML,
    };
    use axum::body::to_bytes;
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;

    #[test]
    fn six_hub_control_assets_are_embedded() {
        for path in [
            "main.js",
            "control_contract.mjs",
            "status_model.mjs",
            "provider_quota_model.mjs",
            "components/Login.js",
            "components/Shell.js",
            "views/Automation.js",
            "views/Workflows.js",
            "views/Capabilities.js",
            "views/NativeCapabilities.js",
            "views/Status.js",
        ] {
            let content = embedded_app_asset(path)
                .unwrap_or_else(|| panic!("Control asset {path} is not embedded"));
            assert!(!content.trim().is_empty(), "Control asset {path} is empty");
        }
        assert!(embedded_app_asset("views/System.js").is_none());
        assert!(embedded_app_asset("../../config.toml").is_none());
    }

    #[test]
    fn captain_wordmark_asset_is_embedded_and_cache_busted() {
        assert!(LOGO_PNG.starts_with(b"\x89PNG\r\n\x1a\n"));
        let width = u32::from_be_bytes(LOGO_PNG[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(LOGO_PNG[20..24].try_into().unwrap());
        assert_eq!((width, height), (1254, 1254));

        let versioned_path = "/assets/logo.png?rev=wordmark-2";
        assert!(APP_HTML.contains(versioned_path));
        assert!(MANIFEST_JSON.contains(versioned_path));
        assert!(
            TERMINAL_HTML.matches(versioned_path).count() >= 2,
            "terminal auth and topbar must use the current graphical emblem"
        );
        for path in ["components/Login.js", "components/Shell.js"] {
            assert!(
                embedded_app_asset(path)
                    .expect("brand surface should be embedded")
                    .contains(versioned_path),
                "{path} must use the current wordmark revision"
            );
        }
    }

    #[test]
    fn every_web_surface_declares_the_captain_favicon() {
        let favicon = "<link rel=\"icon\" type=\"image/png\" sizes=\"1254x1254\" href=\"/assets/logo.png?rev=wordmark-2\">";
        let touch_icon = "<link rel=\"apple-touch-icon\" href=\"/assets/logo.png?rev=wordmark-2\">";

        for (surface, html) in [
            ("control", APP_HTML),
            ("terminal", TERMINAL_HTML),
            ("config", CONFIG_HTML),
        ] {
            assert!(html.contains(favicon), "{surface} favicon is not Captain");
            assert!(
                html.contains(touch_icon),
                "{surface} touch icon is not Captain"
            );
        }
    }

    #[tokio::test]
    async fn favicon_endpoint_serves_the_embedded_captain_png() {
        let response = favicon_ico().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        let body = to_bytes(response.into_body(), LOGO_PNG.len() + 1)
            .await
            .expect("favicon body should be readable");
        assert_eq!(body.as_ref(), LOGO_PNG);
    }

    #[test]
    fn web_terminal_uses_unicode11_cell_widths() {
        assert!(TERMINAL_HTML.contains("e.Unicode11Addon=t()"));
        assert!(TERMINAL_HTML.contains("allowProposedApi: true"));
        assert!(TERMINAL_HTML.contains("term.unicode.activeVersion = '11'"));

        let xterm = TERMINAL_HTML
            .find("e.Terminal=void 0")
            .expect("xterm must be embedded");
        let unicode11 = TERMINAL_HTML
            .find("e.Unicode11Addon=t()")
            .expect("Unicode 11 addon must be embedded");
        let terminal_app = TERMINAL_HTML
            .find("Captain Web Terminal")
            .expect("terminal application must be embedded");
        assert!(xterm < unicode11 && unicode11 < terminal_app);
    }

    #[test]
    fn web_terminal_reopens_only_validated_persisted_sessions() {
        assert!(TERMINAL_HTML.contains("body: JSON.stringify({ activate: false })"));
        assert!(TERMINAL_HTML.contains("New persisted session"));
        assert!(TERMINAL_HTML.contains("bindKnownPersistedSession(items)"));
        assert!(TERMINAL_HTML.contains("fetch('/api/sessions'"));
        assert!(TERMINAL_HTML.contains("return items;"));
        assert!(TERMINAL_HTML.contains("historyById[id] || remoteById[id]"));
        assert!(TERMINAL_HTML.contains("meta.source === 'history' ? makeAutoSessionId() : id"));
        assert!(
            !TERMINAL_HTML.contains(
                "if (!activeResumeSessionId && validUuid(querySession)) activeResumeSessionId = querySession"
            ),
            "a terminal UUID must never be assumed to be a persisted session UUID"
        );
        assert!(
            !TERMINAL_HTML.contains("return items.slice(0, recentSessionsLimit)"),
            "persisted history must not be truncated to the browser recent-session cache"
        );
    }
}

pub async fn embed_chat_js() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                "application/javascript; charset=utf-8",
            ),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        include_str!("../static/js/embed_chat.js"),
    )
}

/// GET /terminal — Serve the embedded xterm.js Captain terminal.
pub async fn terminal_page() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::ETAG, ETAG),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        TERMINAL_HTML,
    )
}

/// GET /config — Serve the embedded Captain config editor.
pub async fn config_page() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::ETAG, ETAG),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        CONFIG_HTML,
    )
}

/// Dedicated terminal page assembled from local, vendored assets only.
///
/// xterm.js guidance explicitly warns against CDN/third-party resources on
/// pages that expose terminal I/O; keep this surface self-contained.
const TERMINAL_HTML: &str = concat!(
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<title>Captain Terminal</title>\n",
    "<link rel=\"manifest\" href=\"/manifest.json\">\n",
    "<link rel=\"icon\" type=\"image/png\" sizes=\"1254x1254\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<link rel=\"apple-touch-icon\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<style>\n",
    include_str!("../static/css/theme.css"),
    "\n",
    include_str!("../static/vendor/xterm/xterm.css"),
    "\n",
    include_str!("../static/css/terminal.css"),
    "\n</style>\n",
    "</head>\n<body class=\"terminal-body\" data-theme=\"dark\">\n",
    include_str!("../static/terminal_body.html"),
    "<script>\n",
    include_str!("../static/vendor/xterm/xterm.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/vendor/xterm/addon-unicode11.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/vendor/xterm/addon-fit.js"),
    "\n</script>\n",
    "<script>\n",
    include_str!("../static/js/pages/terminal.js"),
    "\n</script>\n",
    "</body></html>"
);

/// Dedicated config page assembled from local assets only.
const CONFIG_HTML: &str = concat!(
    "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n",
    "<meta charset=\"utf-8\">\n",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    "<title>Captain Config</title>\n",
    "<link rel=\"manifest\" href=\"/manifest.json\">\n",
    "<link rel=\"icon\" type=\"image/png\" sizes=\"1254x1254\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<link rel=\"apple-touch-icon\" href=\"/assets/logo.png?rev=wordmark-2\">\n",
    "<style>\n",
    include_str!("../static/css/theme.css"),
    "\n",
    include_str!("../static/css/config.css"),
    "\n</style>\n",
    "</head>\n<body class=\"config-body\" data-theme=\"dark\">\n",
    include_str!("../static/config_body.html"),
    "<script>\n",
    include_str!("../static/js/pages/config.js"),
    "\n</script>\n",
    "</body></html>"
);
