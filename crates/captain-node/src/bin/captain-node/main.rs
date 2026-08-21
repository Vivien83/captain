mod cli;
mod render;
mod runtime;
#[cfg(target_os = "windows")]
mod windows_service;

use clap::Parser;

#[tokio::main]
async fn main() {
    init_tracing();
    if let Err(error) = runtime::execute(cli::Cli::parse()).await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("captain_node=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
