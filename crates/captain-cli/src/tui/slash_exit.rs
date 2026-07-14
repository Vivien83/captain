pub(crate) fn is_exit_command(command: &str) -> bool {
    matches!(command, "/exit" | "/quit")
}

#[cfg(test)]
#[path = "slash_exit/tests.rs"]
mod tests;
