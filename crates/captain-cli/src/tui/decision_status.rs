pub(crate) fn decision_message(id: &str, approved: bool) -> String {
    format!("{id} {}", if approved { "approuvé" } else { "refusé" })
}

#[cfg(test)]
#[path = "decision_status/tests.rs"]
mod tests;
