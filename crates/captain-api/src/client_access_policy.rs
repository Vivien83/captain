//! Fail-closed API surface available to paired lightweight Clients.
//!
//! Client device credentials are deliberately not operator credentials. Every
//! route and HTTP method exposed here is reviewed explicitly; new API routes
//! remain unavailable until they are added to this versioned policy.

use axum::http::Method;

pub(crate) const CLIENT_ACCESS_POLICY_VERSION: u16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientTurnProvenance {
    pub(crate) sender_id: Option<String>,
    pub(crate) sender_name: Option<String>,
    pub(crate) channel_type: Option<String>,
    pub(crate) paired_client: bool,
}

impl ClientTurnProvenance {
    pub(crate) fn resolve(
        paired_device_id: Option<&str>,
        surface: &str,
        requested_sender_id: Option<&str>,
        requested_sender_name: Option<&str>,
        requested_channel: Option<&str>,
        default_channel: &str,
    ) -> Self {
        if let Some(device_id) = paired_device_id {
            let device_id = device_id
                .chars()
                .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
                .take(64)
                .collect::<String>();
            return Self {
                sender_id: Some(format!("paired-client-device:{device_id}")),
                sender_name: Some("Paired Client".to_string()),
                channel_type: Some(captain_runtime::client_authority::paired_client_origin(
                    surface,
                )),
                paired_client: true,
            };
        }
        Self {
            sender_id: requested_sender_id.map(str::to_string),
            sender_name: requested_sender_name.map(str::to_string),
            channel_type: Some(requested_channel.unwrap_or(default_channel).to_string()),
            paired_client: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl ClientMethod {
    fn matches(self, method: &Method) -> bool {
        matches!(
            (self, method),
            (Self::Get, &Method::GET)
                | (Self::Post, &Method::POST)
                | (Self::Put, &Method::PUT)
                | (Self::Patch, &Method::PATCH)
                | (Self::Delete, &Method::DELETE)
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct ClientEndpoint {
    methods: &'static [ClientMethod],
    template: &'static str,
}

const GET: &[ClientMethod] = &[ClientMethod::Get];
const POST: &[ClientMethod] = &[ClientMethod::Post];
const PUT: &[ClientMethod] = &[ClientMethod::Put];
const DELETE: &[ClientMethod] = &[ClientMethod::Delete];
const GET_POST: &[ClientMethod] = &[ClientMethod::Get, ClientMethod::Post];
const GET_DELETE: &[ClientMethod] = &[ClientMethod::Get, ClientMethod::Delete];
const GET_PUT: &[ClientMethod] = &[ClientMethod::Get, ClientMethod::Put];
const PATCH_DELETE: &[ClientMethod] = &[ClientMethod::Patch, ClientMethod::Delete];
const GET_PATCH_DELETE: &[ClientMethod] =
    &[ClientMethod::Get, ClientMethod::Patch, ClientMethod::Delete];
const GET_PUT_DELETE: &[ClientMethod] =
    &[ClientMethod::Get, ClientMethod::Put, ClientMethod::Delete];

const CLIENT_ENDPOINTS: &[ClientEndpoint] = &[
    // Status and bounded Live Runs.
    endpoint(GET, "/api/status"),
    endpoint(GET, "/api/health/detail"),
    endpoint(POST, "/api/auth/realtime-ticket"),
    endpoint(GET, "/api/events"),
    endpoint(GET, "/api/execution-targets"),
    endpoint(GET, "/api/tool-runs"),
    endpoint(GET, "/api/tool-runs/*"),
    endpoint(GET, "/api/tool-runs/*/tail"),
    endpoint(POST, "/api/tool-runs/*/cancel"),
    // Agent chat, shared sessions, model selection, and workspace artifacts.
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
    endpoint(GET, "/api/agents/*/reasoning"),
    endpoint(GET, "/api/agents/*/tools"),
    endpoint(GET, "/api/agents/*/skills"),
    endpoint(GET, "/api/agents/*/mcp_servers"),
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
    // Project work. Provider secrets remain outside this surface.
    endpoint(GET_POST, "/api/projects"),
    endpoint(GET, "/api/projects/*/runtime"),
    endpoint(POST, "/api/projects/*/runtime/start"),
    endpoint(POST, "/api/projects/*/runtime/pause"),
    endpoint(POST, "/api/projects/*/runtime/resume"),
    endpoint(POST, "/api/projects/*/runtime/answer"),
    endpoint(POST, "/api/projects/*/runtime/tool-request"),
    endpoint(POST, "/api/projects/*/runtime/takeover"),
    endpoint(GET_PATCH_DELETE, "/api/projects/*"),
    endpoint(GET_PUT, "/api/projects/*/execution-target"),
    endpoint(POST, "/api/projects/*/archive"),
    endpoint(GET, "/api/projects/*/resume"),
    endpoint(&[ClientMethod::Patch], "/api/projects/*/lifecycle"),
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
    // Workflows are editable. Other automation catalogs remain observable but
    // cannot be changed or fired with a Client credential.
    endpoint(GET, "/api/triggers"),
    endpoint(GET, "/api/triggers/*"),
    endpoint(GET, "/api/file-triggers"),
    endpoint(GET, "/api/file-triggers/*"),
    endpoint(GET, "/api/schedules"),
    endpoint(GET, "/api/schedules/*"),
    endpoint(GET_POST, "/api/workflows"),
    endpoint(GET_PUT_DELETE, "/api/workflows/*"),
    endpoint(POST, "/api/workflows/*/run"),
    endpoint(GET, "/api/workflows/*/runs"),
    // Learning and memory. Migration and outbound digest delivery are admin-only.
    endpoint(GET, "/api/templates"),
    endpoint(GET, "/api/templates/*"),
    endpoint(GET, "/api/memory/agents/*/kv"),
    endpoint(GET_PUT_DELETE, "/api/memory/agents/*/kv/*"),
    endpoint(GET, "/api/learning/committed"),
    endpoint(GET, "/api/learning/review"),
    endpoint(GET, "/api/learning/metrics"),
    endpoint(GET, "/api/learning/status"),
    endpoint(GET, "/api/learning/workflows"),
    endpoint(GET, "/api/skills/proposals"),
    endpoint(GET, "/api/skills/patterns"),
    endpoint(GET, "/api/skills/metrics"),
    endpoint(GET, "/api/graph/stats"),
    endpoint(GET, "/api/graph/entities"),
    endpoint(GET, "/api/graph/facts"),
    endpoint(GET_DELETE, "/api/graph/entity/*"),
    endpoint(POST, "/api/graph/fact/*/invalidate"),
    endpoint(GET, "/api/graph/search"),
    endpoint(POST, "/api/graph/dream"),
    endpoint(GET, "/api/memory/events"),
    endpoint(GET, "/api/graph/consciousness"),
    endpoint(GET, "/api/graph/consciousness/digest"),
    endpoint(GET, "/api/consciousness/mood"),
    endpoint(GET, "/api/consciousness/state"),
    endpoint(GET, "/api/consciousness/neuromodulators"),
    // One-shot/session approvals and read-only usage/budget telemetry. A
    // paired credential cannot create, clear or revoke persistent authority.
    endpoint(GET, "/api/approvals"),
    endpoint(POST, "/api/approvals/*/approve"),
    endpoint(POST, "/api/approvals/*/reject"),
    endpoint(POST, "/api/approvals/*/reject_session"),
    endpoint(POST, "/api/approvals/*/approve_session"),
    endpoint(GET, "/api/usage"),
    endpoint(GET, "/api/usage/summary"),
    endpoint(GET, "/api/usage/by-model"),
    endpoint(GET, "/api/usage/daily"),
    endpoint(GET, "/api/budget"),
    endpoint(GET, "/api/budget/agents"),
    endpoint(GET, "/api/budget/agents/*"),
    // Immutable artifacts and read-only catalogs needed by work surfaces.
    endpoint(GET, "/api/artifacts"),
    endpoint(GET, "/api/artifacts/*"),
    endpoint(GET, "/api/artifacts/*/versions"),
    endpoint(GET, "/api/artifacts/*/versions/*/download"),
    endpoint(GET, "/api/artifacts/*/versions/*/preview"),
    endpoint(GET, "/api/tools"),
    endpoint(GET, "/api/skills"),
    endpoint(GET, "/api/profiles"),
    endpoint(GET, "/api/models"),
    endpoint(GET, "/api/models/updates"),
    endpoint(GET, "/api/models/aliases"),
    endpoint(GET, "/api/providers"),
    endpoint(GET, "/api/capabilities/native"),
    endpoint(GET, "/api/capabilities/native/runs"),
    endpoint(GET, "/api/capabilities/native/runs/*"),
    endpoint(GET, "/api/capabilities/native/*"),
];

const fn endpoint(methods: &'static [ClientMethod], template: &'static str) -> ClientEndpoint {
    ClientEndpoint { methods, template }
}

pub(crate) fn allows(method: &Method, path: &str) -> bool {
    captain_wire::client_api_path_is_authorizable(path)
        && CLIENT_ENDPOINTS.iter().any(|rule| {
            rule.methods.iter().any(|allowed| allowed.matches(method))
                && path_matches(rule.template, path)
        })
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
    fn work_surfaces_are_explicitly_available() {
        for (method, path) in [
            (Method::GET, "/api/status"),
            (Method::POST, "/api/auth/realtime-ticket"),
            (Method::POST, "/api/agents/captain/message/stream"),
            (Method::DELETE, "/api/sessions/session-1"),
            (Method::GET, "/api/execution-targets"),
            (Method::PUT, "/api/sessions/session-1/execution-target"),
            (Method::PUT, "/api/projects/project-1/execution-target"),
            (Method::POST, "/api/projects/project-1/runtime/start"),
            (Method::PUT, "/api/workflows/workflow-1"),
            (Method::GET, "/api/learning/review"),
            (Method::GET, "/api/artifacts/artifact-1/versions/2/preview"),
            (Method::POST, "/api/tool-runs/run-1/cancel"),
            (Method::POST, "/api/approvals/approval-1/approve"),
        ] {
            assert!(allows(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn administrative_and_future_routes_fail_closed() {
        for (method, path) in [
            (Method::GET, "/api/config"),
            (Method::POST, "/api/config/set"),
            (Method::POST, "/api/providers/openai/key"),
            (Method::POST, "/api/shutdown"),
            (Method::GET, "/api/hub/devices"),
            (Method::POST, "/api/hub/pairing/enrollment"),
            (Method::POST, "/api/agents/captain/api/token/rotate"),
            (Method::PUT, "/api/agents/captain/model"),
            (Method::PUT, "/api/agents/captain/reasoning"),
            (Method::POST, "/api/agents/captain/model-switch/apply"),
            (Method::GET, "/api/agents/captain/files"),
            (Method::POST, "/api/projects/launch"),
            (Method::GET, "/api/projects/environment"),
            (Method::GET, "/api/projects/github/repos"),
            (Method::PUT, "/api/projects/github/token"),
            (Method::DELETE, "/api/processes/process-1"),
            (Method::POST, "/api/skills/uninstall"),
            (Method::POST, "/api/triggers"),
            (Method::PUT, "/api/triggers/trigger-1"),
            (Method::POST, "/api/schedules/schedule-1/run"),
            (Method::POST, "/api/learning/review/review-1/decide"),
            (Method::POST, "/api/learning/workflows/proposal-1/decide"),
            (Method::POST, "/api/skills/proposals/proposal-1/decide"),
            (Method::POST, "/api/memory/migrate"),
            (Method::POST, "/api/capabilities/native/install"),
            (Method::POST, "/api/update/install"),
            (Method::POST, "/api/approvals/a/approve_always"),
            (Method::POST, "/api/approvals/a/reject_always"),
            (Method::POST, "/api/approvals/clear_session"),
            (Method::DELETE, "/api/approvals/rules/rule-1"),
            (Method::GET, "/api/future/operator-secret"),
        ] {
            assert!(!allows(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn wrong_methods_and_prefix_collisions_are_rejected() {
        for (method, path) in [
            (Method::POST, "/api/status"),
            (Method::DELETE, "/api/models"),
            (Method::GET, "/api/tool-runs/run-1/cancel"),
            (Method::GET, "/api/status/private"),
            (Method::GET, "/api/agents/captain/api"),
            (Method::GET, "/api/agents/captain/files/a/b"),
            (Method::GET, "/api/projects/github/token"),
            (Method::GET, "/api/status/"),
        ] {
            assert!(!allows(&method, path), "{method} {path}");
        }
    }

    #[test]
    fn policy_is_versioned_and_templates_are_exact_segment_patterns() {
        assert_eq!(CLIENT_ACCESS_POLICY_VERSION, 5);
        assert!(CLIENT_ENDPOINTS.iter().all(|rule| {
            rule.template.starts_with('/')
                && !rule.template.ends_with('/')
                && !rule.template.contains("**")
                && !rule.template.contains("//")
        }));
    }

    #[test]
    fn paired_identity_overrides_forgeable_message_metadata() {
        let provenance = ClientTurnProvenance::resolve(
            Some("device-123"),
            "api",
            Some("operator"),
            Some("Administrator"),
            Some("telegram"),
            "web",
        );

        assert!(provenance.paired_client);
        assert_eq!(
            provenance.sender_id.as_deref(),
            Some("paired-client-device:device-123")
        );
        assert_eq!(provenance.sender_name.as_deref(), Some("Paired Client"));
        assert_eq!(
            provenance.channel_type.as_deref(),
            Some("paired-client:api")
        );
    }

    #[test]
    fn operator_message_metadata_is_preserved() {
        let provenance = ClientTurnProvenance::resolve(
            None,
            "api",
            Some("operator"),
            Some("Vivien"),
            Some("cli"),
            "web",
        );

        assert!(!provenance.paired_client);
        assert_eq!(provenance.sender_id.as_deref(), Some("operator"));
        assert_eq!(provenance.sender_name.as_deref(), Some("Vivien"));
        assert_eq!(provenance.channel_type.as_deref(), Some("cli"));
    }
}
