//! Operational CLI for one outbound-only local execution Node.

mod pair;
mod run;
mod status;
pub(crate) mod support;

use crate::{ui, NodeCommands};
use std::{future::Future, path::PathBuf};

pub(crate) fn cmd_node(config_path: Option<PathBuf>, command: NodeCommands) {
    let result = match command {
        NodeCommands::Pair(args) => block_on(pair::pair_node(pair::PairRequest {
            hub: args.hub,
            workspace: args.workspace,
            workspace_id: args.workspace_id,
            name: args.name,
            label: args.label,
            allow_mutation: args.allow_mutation,
            ca_bundle: args.ca_bundle,
            proxy: args.proxy,
            proxy_username: args.proxy_username,
            proxy_password_secret: args.proxy_password_secret,
            no_proxy: args.no_proxy,
            no_browser: args.no_browser,
        })),
        NodeCommands::Run => block_on(run::run_node(config_path)),
        NodeCommands::Status { json } => status::node_status(json),
        NodeCommands::Reset { yes } => status::reset_node(yes),
    };

    if let Err(error) = result {
        ui::error(&error);
        std::process::exit(1);
    }
}

fn block_on<F>(future: F) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|_| "The local Node async runtime could not start".to_string())?
        .block_on(future)
}
