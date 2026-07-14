#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRequestDecision {
    Approve,
    Deny,
}

impl ToolRequestDecision {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "approve" | "allow" | "allow-once" | "allow_once" => Some(Self::Approve),
            "deny" | "reject" => Some(Self::Deny),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }

    pub(crate) fn as_status(self) -> &'static str {
        match self {
            Self::Approve => "approved",
            Self::Deny => "denied",
        }
    }

    pub(crate) fn default_detail(self) -> &'static str {
        match self {
            Self::Approve => "The operator approved the requested tools for this phase.",
            Self::Deny => "The operator denied the requested tools for this phase.",
        }
    }
}
