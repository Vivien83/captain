//! Lightweight Captain Console core.
//!
//! This crate contains no agent loop, provider, memory database, channel or
//! local execution engine. It binds one immutable paired Captain authority to
//! a private loopback gateway for terminal and Desktop surfaces.

mod gateway;
mod manager;
mod observation;
mod pairing;
mod profiles;
mod secret_support;
mod tui;

pub use captain_node::ClientPairingProgress;
pub use gateway::{start_gateway, start_gateway_for_profile, GatewayError, GatewayHandle};
pub use manager::{ConsoleLaunch, ConsoleManager, ConsoleManagerError};
pub use observation::{
    ConsoleAuthorityAvailability, ConsoleAuthorityObservation, ConsoleQuotaObservation,
};
pub use pairing::{
    ConsolePairingError, ConsolePairingOptions, ConsolePairingSession, PAIRING_POLL_INTERVAL,
};
pub use profiles::{ConsoleProfileCatalog, ConsoleProfileError, ConsoleProfileSummary};
pub use tui::{run_tui, ConsoleTuiError};
