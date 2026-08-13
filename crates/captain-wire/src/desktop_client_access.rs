//! Narrow HTTP relay contract for the paired Desktop Client.
//!
//! The Hub reapplies its own Client policy. This additional allowlist limits
//! what the local gateway can relay at all to explicitly authorized work data.

pub const DESKTOP_CLIENT_POLICY_VERSION: u16 = 2;

/// Reject path spellings that an HTTP client or reverse proxy could normalize
/// into a different authorization target. Query strings are validated by the
/// caller and must not be included here.
pub fn client_relay_path_is_canonical(path: &str) -> bool {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("//")
        || path.contains('\\')
        || path.contains('?')
        || path.contains('#')
        || (path.len() > 1 && path.ends_with('/'))
    {
        return false;
    }

    for segment in path[1..].split('/') {
        if segment == "." || segment == ".." || !percent_encoding_is_canonical(segment) {
            return false;
        }
    }
    true
}

/// Shared semantic guard for API namespaces where a dynamic work identifier
/// collides with an operator-only static route.
pub fn client_api_path_is_authorizable(path: &str) -> bool {
    if !client_relay_path_is_canonical(path) {
        return false;
    }
    let mut segments = path.trim_start_matches('/').split('/');
    if segments.next() != Some("api") || segments.next() != Some("projects") {
        return true;
    }
    let Some(project_segment) = segments.next() else {
        return true;
    };
    !["environment", "github", "launch"]
        .iter()
        .any(|reserved| percent_decoded_ascii_eq(project_segment, reserved))
}

fn percent_decoded_ascii_eq(segment: &str, expected: &str) -> bool {
    let source = segment.as_bytes();
    let expected = expected.as_bytes();
    let mut source_index = 0;
    let mut expected_index = 0;
    while source_index < source.len() && expected_index < expected.len() {
        let decoded = if source[source_index] == b'%' {
            let (Some(high), Some(low)) = (
                source
                    .get(source_index + 1)
                    .and_then(|byte| hex_value(*byte)),
                source
                    .get(source_index + 2)
                    .and_then(|byte| hex_value(*byte)),
            ) else {
                return false;
            };
            source_index += 3;
            (high << 4) | low
        } else {
            let decoded = source[source_index];
            source_index += 1;
            decoded
        };
        if decoded != expected[expected_index] {
            return false;
        }
        expected_index += 1;
    }
    source_index == source.len() && expected_index == expected.len()
}

fn percent_encoding_is_canonical(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        let Some(high) = bytes.get(index + 1).and_then(|byte| hex_value(*byte)) else {
            return false;
        };
        let Some(low) = bytes.get(index + 2).and_then(|byte| hex_value(*byte)) else {
            return false;
        };
        let decoded = (high << 4) | low;
        if matches!(
            decoded,
            b'.' | b'/' | b'\\' | b'%' | b'?' | b'#' | 0..=31 | 127
        ) {
            return false;
        }
        index += 3;
    }
    true
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientHttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, Copy)]
struct ClientEndpoint {
    methods: &'static [ClientHttpMethod],
    template: &'static str,
}

const GET: &[ClientHttpMethod] = &[ClientHttpMethod::Get];
const POST: &[ClientHttpMethod] = &[ClientHttpMethod::Post];
const PUT: &[ClientHttpMethod] = &[ClientHttpMethod::Put];
const DELETE: &[ClientHttpMethod] = &[ClientHttpMethod::Delete];
const GET_POST: &[ClientHttpMethod] = &[ClientHttpMethod::Get, ClientHttpMethod::Post];
const GET_DELETE: &[ClientHttpMethod] = &[ClientHttpMethod::Get, ClientHttpMethod::Delete];
const GET_PUT: &[ClientHttpMethod] = &[ClientHttpMethod::Get, ClientHttpMethod::Put];
const PATCH_DELETE: &[ClientHttpMethod] = &[ClientHttpMethod::Patch, ClientHttpMethod::Delete];
const GET_PATCH_DELETE: &[ClientHttpMethod] = &[
    ClientHttpMethod::Get,
    ClientHttpMethod::Patch,
    ClientHttpMethod::Delete,
];
const GET_PUT_DELETE: &[ClientHttpMethod] = &[
    ClientHttpMethod::Get,
    ClientHttpMethod::Put,
    ClientHttpMethod::Delete,
];

const ENDPOINTS: &[ClientEndpoint] = &[
    // Authentication handoff, telemetry and bounded Live Runs.
    endpoint(GET, "/api/auth/check"),
    endpoint(POST, "/api/auth/realtime-ticket"),
    endpoint(GET, "/api/status"),
    endpoint(GET, "/api/execution-targets"),
    endpoint(GET, "/api/health/detail"),
    endpoint(GET, "/api/usage"),
    endpoint(GET, "/api/usage/summary"),
    endpoint(GET, "/api/usage/by-model"),
    endpoint(GET, "/api/usage/daily"),
    endpoint(GET, "/api/budget"),
    endpoint(GET, "/api/budget/agents"),
    endpoint(GET, "/api/budget/agents/*"),
    endpoint(GET, "/api/tool-runs"),
    endpoint(GET, "/api/tool-runs/*"),
    endpoint(GET, "/api/tool-runs/*/tail"),
    endpoint(POST, "/api/tool-runs/*/cancel"),
    // Chats and shared sessions.
    endpoint(GET, "/api/agents"),
    endpoint(GET, "/api/agents/*"),
    endpoint(POST, "/api/agents/*/message"),
    endpoint(POST, "/api/agents/*/message/stream"),
    endpoint(POST, "/api/agents/*/message/answer"),
    endpoint(GET, "/api/agents/*/session"),
    endpoint(GET_POST, "/api/agents/*/sessions"),
    endpoint(POST, "/api/agents/*/sessions/*/switch"),
    endpoint(POST, "/api/agents/*/session/reset"),
    endpoint(POST, "/api/agents/*/session/restore"),
    endpoint(DELETE, "/api/agents/*/history"),
    endpoint(POST, "/api/agents/*/session/compact"),
    endpoint(GET, "/api/agents/*/ws"),
    endpoint(POST, "/api/agents/*/interrupt"),
    endpoint(GET, "/api/agents/*/deliveries"),
    endpoint(POST, "/api/agents/*/upload"),
    endpoint(GET_POST, "/api/agents/*/feedback"),
    endpoint(GET, "/api/sessions"),
    endpoint(GET_DELETE, "/api/sessions/*"),
    endpoint(PUT, "/api/sessions/*/label"),
    endpoint(GET, "/api/sessions/*/events"),
    endpoint(GET_PUT, "/api/sessions/*/execution-target"),
    endpoint(GET, "/api/agents/*/sessions/by-label/*"),
    endpoint(GET, "/api/uploads/*"),
    // Project data. Hub takeover is deliberately absent.
    endpoint(GET_POST, "/api/projects"),
    endpoint(GET_PATCH_DELETE, "/api/projects/*"),
    endpoint(GET_PUT, "/api/projects/*/execution-target"),
    endpoint(POST, "/api/projects/*/archive"),
    endpoint(GET, "/api/projects/*/resume"),
    endpoint(&[ClientHttpMethod::Patch], "/api/projects/*/lifecycle"),
    endpoint(GET, "/api/projects/*/runtime"),
    endpoint(POST, "/api/projects/*/runtime/start"),
    endpoint(POST, "/api/projects/*/runtime/pause"),
    endpoint(POST, "/api/projects/*/runtime/resume"),
    endpoint(POST, "/api/projects/*/runtime/answer"),
    endpoint(POST, "/api/projects/*/runtime/tool-request"),
    endpoint(GET_POST, "/api/projects/*/goals"),
    endpoint(PATCH_DELETE, "/api/projects/*/goals/*"),
    endpoint(POST, "/api/projects/*/goals/*/pause"),
    endpoint(POST, "/api/projects/*/goals/*/resume"),
    endpoint(GET_POST, "/api/projects/*/tasks"),
    endpoint(PATCH_DELETE, "/api/project-tasks/*"),
    endpoint(GET_POST, "/api/projects/*/milestones"),
    endpoint(GET, "/api/projects/*/milestones/progress"),
    endpoint(POST, "/api/milestones/*/complete"),
    endpoint(GET_POST, "/api/projects/*/checkpoints"),
    endpoint(GET_PUT_DELETE, "/api/active-project/*"),
    // Workflows only; triggers, schedules and webhooks are outside this Client.
    endpoint(GET_POST, "/api/workflows"),
    endpoint(GET_PUT_DELETE, "/api/workflows/*"),
    endpoint(POST, "/api/workflows/*/run"),
    endpoint(GET, "/api/workflows/*/runs"),
    // Explicit memory KV and its sanitized event stream.
    endpoint(GET, "/api/memory/agents/*/kv"),
    endpoint(GET_PUT_DELETE, "/api/memory/agents/*/kv/*"),
    endpoint(GET, "/api/memory/events"),
    // One-shot and session approvals. Persistent "always" rules are absent.
    endpoint(GET, "/api/approvals"),
    endpoint(POST, "/api/approvals/*/approve"),
    endpoint(POST, "/api/approvals/*/reject"),
    endpoint(POST, "/api/approvals/*/reject_session"),
    endpoint(POST, "/api/approvals/*/approve_session"),
    // Immutable work products.
    endpoint(GET, "/api/artifacts"),
    endpoint(GET, "/api/artifacts/*"),
    endpoint(GET, "/api/artifacts/*/versions"),
    endpoint(GET, "/api/artifacts/*/versions/*/download"),
    endpoint(GET, "/api/artifacts/*/versions/*/preview"),
];

const fn endpoint(methods: &'static [ClientHttpMethod], template: &'static str) -> ClientEndpoint {
    ClientEndpoint { methods, template }
}

pub fn desktop_client_route_allows(method: ClientHttpMethod, path: &str) -> bool {
    client_api_path_is_authorizable(path)
        && ENDPOINTS
            .iter()
            .any(|rule| rule.methods.contains(&method) && path_matches(rule.template, path))
}

fn path_matches(template: &str, path: &str) -> bool {
    if !template.starts_with('/')
        || !path.starts_with('/')
        || template.ends_with('/')
        || path.ends_with('/')
    {
        return false;
    }
    let mut expected = template[1..].split('/');
    let mut actual = path[1..].split('/');
    loop {
        match (expected.next(), actual.next()) {
            (Some("*"), Some(value)) if !value.is_empty() => {}
            (Some(left), Some(right)) if left == right => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicitly_authorized_work_data_is_relayable() {
        for (method, path) in [
            (ClientHttpMethod::Get, "/api/status"),
            (ClientHttpMethod::Post, "/api/auth/realtime-ticket"),
            (ClientHttpMethod::Get, "/api/agents/captain/ws"),
            (ClientHttpMethod::Put, "/api/sessions/session-1/label"),
            (ClientHttpMethod::Get, "/api/execution-targets"),
            (
                ClientHttpMethod::Put,
                "/api/projects/project-1/execution-target",
            ),
            (
                ClientHttpMethod::Post,
                "/api/projects/project-1/runtime/start",
            ),
            (ClientHttpMethod::Put, "/api/workflows/workflow-1"),
            (
                ClientHttpMethod::Put,
                "/api/memory/agents/captain/kv/preference",
            ),
            (ClientHttpMethod::Post, "/api/approvals/approval-1/approve"),
            (ClientHttpMethod::Post, "/api/tool-runs/run-1/cancel"),
            (
                ClientHttpMethod::Get,
                "/api/artifacts/artifact-1/versions/2/preview",
            ),
        ] {
            assert!(
                desktop_client_route_allows(method, path),
                "{method:?} {path}"
            );
        }
    }

    #[test]
    fn administration_extensions_and_persistent_authority_never_relay() {
        for (method, path) in [
            (ClientHttpMethod::Get, "/api/config"),
            (ClientHttpMethod::Post, "/api/providers/openai/key"),
            (ClientHttpMethod::Post, "/api/shutdown"),
            (ClientHttpMethod::Get, "/api/hub/devices"),
            (ClientHttpMethod::Post, "/api/hub/pairing/enrollment"),
            (ClientHttpMethod::Put, "/api/agents/captain/reasoning"),
            (ClientHttpMethod::Get, "/api/agents/captain/files"),
            (
                ClientHttpMethod::Post,
                "/api/projects/project-1/runtime/takeover",
            ),
            (ClientHttpMethod::Get, "/api/learning/review"),
            (ClientHttpMethod::Get, "/api/graph/facts"),
            (ClientHttpMethod::Get, "/api/tools"),
            (ClientHttpMethod::Get, "/api/skills"),
            (ClientHttpMethod::Post, "/api/approvals/a/approve_always"),
            (ClientHttpMethod::Post, "/api/approvals/a/reject_always"),
            (ClientHttpMethod::Delete, "/api/approvals/rules/rule-1"),
            (ClientHttpMethod::Post, "/api/update/install"),
        ] {
            assert!(
                !desktop_client_route_allows(method, path),
                "{method:?} {path}"
            );
        }
    }

    #[test]
    fn wrong_methods_and_prefix_collisions_fail_closed() {
        for (method, path) in [
            (ClientHttpMethod::Post, "/api/status"),
            (ClientHttpMethod::Delete, "/api/artifacts"),
            (ClientHttpMethod::Get, "/api/tool-runs/run-1/cancel"),
            (ClientHttpMethod::Get, "/api/status/private"),
            (ClientHttpMethod::Get, "/api/status/"),
        ] {
            assert!(
                !desktop_client_route_allows(method, path),
                "{method:?} {path}"
            );
        }
    }

    #[test]
    fn ambiguous_or_normalizable_paths_fail_closed() {
        for path in [
            "/assets/app/../api/status",
            "/assets/app/%2e%2e/api/status",
            "/assets/app/%252e%252e/api/status",
            "/assets/app%2f..%2fapi/status",
            "/assets/app\\..\\api/status",
            "/api//status",
            "/api/status/",
            "/api/status%00",
            "/api/status%",
        ] {
            assert!(!client_relay_path_is_canonical(path), "{path}");
            assert!(!desktop_client_route_allows(ClientHttpMethod::Get, path));
        }
        assert!(client_relay_path_is_canonical(
            "/api/sessions/by-label/Mon%20projet"
        ));
        assert!(client_relay_path_is_canonical("/assets/app/main.js"));
        assert!(client_relay_path_is_canonical("/"));
    }

    #[test]
    fn reserved_project_namespaces_never_match_dynamic_project_rules() {
        for path in [
            "/api/projects/environment",
            "/api/projects/%65nvironment",
            "/api/projects/github",
            "/api/projects/g%69thub/repos",
            "/api/projects/github/repos",
            "/api/projects/launch",
        ] {
            assert!(client_relay_path_is_canonical(path));
            assert!(!client_api_path_is_authorizable(path));
            assert!(!desktop_client_route_allows(ClientHttpMethod::Get, path));
        }
        assert!(client_api_path_is_authorizable(
            "/api/projects/customer-environment"
        ));
    }

    #[test]
    fn policy_is_versioned_and_templates_are_exact() {
        assert_eq!(DESKTOP_CLIENT_POLICY_VERSION, 2);
        assert!(ENDPOINTS.iter().all(|rule| {
            rule.template.starts_with('/')
                && !rule.template.ends_with('/')
                && !rule.template.contains("**")
                && !rule.template.contains("//")
        }));
    }
}
