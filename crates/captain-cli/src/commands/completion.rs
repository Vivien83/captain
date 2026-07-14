use clap::CommandFactory;

use crate::Cli;

pub(crate) fn cmd_completion(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "captain", &mut std::io::stdout());
}
