pub(crate) fn is_think_command(command: &str) -> bool {
    command == "/think"
}

#[cfg(test)]
#[path = "slash_think/tests.rs"]
mod tests;
