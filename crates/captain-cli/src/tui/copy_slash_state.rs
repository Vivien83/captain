#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopySlashTarget {
    Command,
    Response,
}

pub(crate) fn copy_slash_target_for_arg(args: &str) -> Option<CopySlashTarget> {
    let normalized_arg = args.trim().to_ascii_lowercase();
    match normalized_arg.as_str() {
        "command" | "cmd" | "commande" => Some(CopySlashTarget::Command),
        "" | "response" => Some(CopySlashTarget::Response),
        "réponse" | "reponse" => Some(CopySlashTarget::Response),
        _ => None,
    }
}

#[cfg(test)]
#[path = "copy_slash_state/tests.rs"]
mod tests;
