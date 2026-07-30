use super::*;

const REVIEWED_PUBLIC_ALLOWLIST: &[PublicEndpoint] = &[
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Prefix("/assets/"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/logo.svg"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/favicon.ico"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/manifest.json"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/sw.js"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/health"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/version"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::Exact("/api/auth/login"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::Exact("/api/auth/logout"),
    },
    PublicEndpoint {
        method: PublicMethod::Get,
        path: PublicPath::Exact("/api/auth/check"),
    },
    PublicEndpoint {
        method: PublicMethod::Post,
        path: PublicPath::AgentApiIngress,
    },
];

#[test]
fn public_allowlist_is_exactly_the_reviewed_matrix() {
    assert_eq!(PUBLIC_ALLOWLIST, REVIEWED_PUBLIC_ALLOWLIST);
}

#[test]
fn reviewed_static_and_boot_routes_are_public_only_for_get() {
    for path in [
        "/",
        "/assets/logo.png",
        "/assets/app/main.js",
        "/logo.svg",
        "/favicon.ico",
        "/manifest.json",
        "/sw.js",
        "/api/health",
        "/api/version",
        "/api/auth/check",
    ] {
        assert!(
            is_public_endpoint(&Method::GET, path),
            "{path} must be public for GET"
        );
        assert!(
            !is_public_endpoint(&Method::POST, path),
            "{path} must not bypass auth for POST"
        );
    }
}

#[test]
fn reviewed_auth_mutations_are_public_only_for_post() {
    for path in ["/api/auth/login", "/api/auth/logout"] {
        assert!(is_public_endpoint(&Method::POST, path));
        assert!(!is_public_endpoint(&Method::GET, path));
    }
}

#[test]
fn agent_ingress_bypass_requires_the_exact_typed_route() {
    let valid = "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/ingress";
    assert!(is_public_endpoint(&Method::POST, valid));
    assert!(!is_public_endpoint(&Method::GET, valid));

    for path in [
        "/hooks/agents/not-an-agent/ingress",
        "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/other",
        "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/extra/ingress",
        "/hooks/agents//ingress",
    ] {
        assert!(
            !is_public_endpoint(&Method::POST, path),
            "{path} must use global auth"
        );
    }
}

#[test]
fn every_operational_read_from_the_previous_policy_is_private() {
    for path in [
        "/terminal",
        "/config",
        "/embed/chat.js",
        "/.well-known/agent.json",
        "/a2a/agents",
        "/a2a/tasks/01234567",
        "/api/health/detail",
        "/api/status",
        "/api/agents",
        "/api/profiles",
        "/api/sessions",
        "/api/approvals",
        "/api/approvals/01234567",
        "/api/budget",
        "/api/budget/agents",
        "/api/budget/agents/01234567",
        "/api/providers",
        "/api/models",
        "/api/models/aliases",
        "/api/hands",
        "/api/hands/active",
        "/api/hands/example",
        "/api/skills",
        "/api/channels",
        "/api/workflows",
        "/api/integrations",
        "/api/integrations/available",
        "/api/integrations/health",
        "/api/cron/jobs",
        "/api/network/status",
        "/api/a2a/agents",
        "/api/uploads/example",
        "/api/logs/stream",
        "/api/providers/github-copilot/oauth/poll/example",
    ] {
        assert!(
            !is_public_endpoint(&Method::GET, path),
            "{path} leaked through the public GET matrix"
        );
    }

    assert!(!is_public_endpoint(
        &Method::POST,
        "/api/providers/github-copilot/oauth/start"
    ));
}

#[test]
fn unknown_routes_fail_closed() {
    for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
        assert!(!is_public_endpoint(&method, "/api/future-sensitive-route"));
    }
}
