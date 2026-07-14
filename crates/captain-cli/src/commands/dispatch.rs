use clap::CommandFactory;
use std::path::PathBuf;

use crate::{
    AgentCommands, ApprovalsCommands, AuthCommands, AutonomyCommands, ChannelCommands,
    ChannelDeadLetterCommands, ChannelInboundCommands, Commands, ConfigCommands, CronCommands,
    DevicesCommands, EmbeddingsCommands, GatewayCommands, HandCommands, IntegrationCommands,
    KnownHostsCommands, LoginCommands, MemoryCommands, ModelsCommands, ProcessCommands,
    SecurityCommands, ServiceCommands, SkillCommands, SnapshotCommands, SshCommands,
    SystemCommands, TriggerCommands, VaultCommands, VoiceCommands, WebhooksCommands,
    WorkflowCommands,
};

pub(crate) fn dispatch_cli(cli: crate::Cli) {
    match cli.command {
        None => dispatch_default(cli.config),
        Some(command) => dispatch_command(cli.config, command),
    }
}

fn dispatch_default(config: Option<PathBuf>) {
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        crate::Cli::command().print_help().unwrap();
        println!();
        return;
    }
    crate::tui::run(config);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchFamily {
    Core,
    Work,
    Capability,
    Lifecycle,
}

fn command_family(command: &Commands) -> DispatchFamily {
    match command {
        Commands::Tui
        | Commands::Login(_)
        | Commands::Auth(_)
        | Commands::Init { .. }
        | Commands::Start { .. }
        | Commands::Stop
        | Commands::Chat { .. }
        | Commands::Status { .. }
        | Commands::Doctor { .. }
        | Commands::Terminal
        | Commands::Update { .. }
        | Commands::Completion { .. }
        | Commands::Mcp
        | Commands::Health { .. }
        | Commands::Onboard { .. }
        | Commands::Setup { .. }
        | Commands::Configure
        | Commands::Message { .. }
        | Commands::System(_) => DispatchFamily::Core,
        Commands::Agent(_)
        | Commands::Workflow(_)
        | Commands::Project(_)
        | Commands::Trigger(_)
        | Commands::Approvals(_)
        | Commands::Cron(_)
        | Commands::Autonomy(_)
        | Commands::Sessions { .. }
        | Commands::Replay { .. }
        | Commands::Logs { .. }
        | Commands::Gateway(_)
        | Commands::Service(_)
        | Commands::Process(_)
        | Commands::Webhooks(_) => DispatchFamily::Work,
        Commands::Skill(_)
        | Commands::Channel(_)
        | Commands::Hand(_)
        | Commands::Integration(_)
        | Commands::Voice(_)
        | Commands::Embeddings(_)
        | Commands::Ssh(_)
        | Commands::Config(_)
        | Commands::Add { .. }
        | Commands::Remove { .. }
        | Commands::Integrations { .. }
        | Commands::Vault(_)
        | Commands::New { .. }
        | Commands::Models(_)
        | Commands::Security(_)
        | Commands::Memory(_)
        | Commands::Devices(_)
        | Commands::Qr => DispatchFamily::Capability,
        Commands::Snapshot(_) | Commands::Reset { .. } | Commands::Uninstall { .. } => {
            DispatchFamily::Lifecycle
        }
    }
}

fn dispatch_command(config: Option<PathBuf>, command: Commands) {
    match command_family(&command) {
        DispatchFamily::Core => dispatch_core_command(config, command),
        DispatchFamily::Work => dispatch_work_command(config, command),
        DispatchFamily::Capability => dispatch_capability_command(command),
        DispatchFamily::Lifecycle => dispatch_lifecycle_command(command),
    }
}

fn dispatch_core_command(config: Option<PathBuf>, command: Commands) {
    match command {
        Commands::Tui => crate::tui::run(config),
        Commands::Login(sub) => match sub {
            LoginCommands::Codex { with_model } => super::auth::cmd_login_codex(with_model),
        },
        Commands::Auth(sub) => match sub {
            AuthCommands::Status { json } => super::auth::cmd_auth_status(json),
            AuthCommands::Doctor { json, test } => super::auth::cmd_auth_doctor(json, test),
            AuthCommands::Login { provider } => super::auth::cmd_auth_login(&provider),
        },
        Commands::Init { quick } => super::init::cmd_init(quick),
        Commands::Start { yolo } => super::daemon::cmd_start(config, yolo),
        Commands::Stop => super::daemon::cmd_stop(),
        Commands::Chat { agent, plain } => super::chat::cmd_quick_chat(config, agent, plain),
        Commands::Status { json, verbose } => super::status::cmd_status(config, json, verbose),
        Commands::Doctor {
            json,
            repair,
            full,
            brand_audit,
        } => super::doctor::cmd_doctor(json, repair, full, brand_audit),
        Commands::Terminal => super::terminal::cmd_terminal(),
        Commands::Update {
            check,
            yes,
            version,
        } => super::update::cmd_update(check, yes, version),
        Commands::Completion { shell } => super::completion::cmd_completion(shell),
        Commands::Mcp => crate::mcp::run_mcp_server(config),
        Commands::Health { json } => super::health::cmd_health(json),
        Commands::Onboard { quick } => super::init::cmd_init(quick),
        Commands::Setup {
            target,
            quick,
            non_interactive,
            profile,
            yes,
            from_env,
            answers,
        } => {
            let setup_profile = profile.as_deref().or(target.as_deref());
            if quick || non_interactive || from_env || answers.is_some() {
                super::setup::cmd_setup_non_interactive(
                    setup_profile,
                    yes || quick || from_env || answers.is_some(),
                    from_env || answers.is_some(),
                    answers.as_deref(),
                );
            } else {
                super::setup::cmd_setup_minimal(setup_profile);
            }
        }
        Commands::Configure => super::init::cmd_init(false),
        Commands::Message { agent, text, json } => super::message::cmd_message(&agent, &text, json),
        Commands::System(sub) => match sub {
            SystemCommands::Info { json } => super::system::cmd_system_info(json),
            SystemCommands::Version { json } => super::system::cmd_system_version(json),
        },
        _ => unreachable!("core command routed to wrong dispatch family"),
    }
}

fn dispatch_work_command(config: Option<PathBuf>, command: Commands) {
    match command {
        Commands::Agent(sub) => dispatch_agent_command(config, sub),
        Commands::Workflow(sub) => dispatch_workflow_command(sub),
        Commands::Project(sub) => super::project::cmd_project(sub),
        Commands::Trigger(sub) => dispatch_trigger_command(sub),
        Commands::Approvals(sub) => dispatch_approvals_command(sub),
        Commands::Cron(sub) => dispatch_cron_command(sub),
        Commands::Autonomy(sub) => dispatch_autonomy_command(sub),
        Commands::Sessions {
            command,
            agent,
            json,
        } => super::sessions::cmd_sessions(command, agent, json),
        Commands::Replay {
            session_id,
            events,
            json,
        } => super::replay::cmd_replay(&session_id, events, json),
        Commands::Logs {
            target,
            lines,
            follow,
            since,
            agent,
            channel,
            json,
        } => super::logs::cmd_logs(
            target,
            lines,
            follow,
            since.as_deref(),
            agent.as_deref(),
            channel.as_deref(),
            json,
        ),
        Commands::Gateway(sub) => dispatch_gateway_command(config, sub),
        Commands::Service(sub) => dispatch_service_command(sub),
        Commands::Process(sub) => dispatch_process_command(sub),
        Commands::Webhooks(sub) => dispatch_webhooks_command(sub),
        _ => unreachable!("work command routed to wrong dispatch family"),
    }
}

fn dispatch_agent_command(config: Option<PathBuf>, sub: AgentCommands) {
    match sub {
        AgentCommands::New { template } => super::agent::cmd_agent_new(config, template),
        AgentCommands::Spawn { manifest } => super::agent::cmd_agent_spawn(config, manifest),
        AgentCommands::List { json } => super::agent::cmd_agent_list(config, json),
        AgentCommands::Caps { agent_id, json } => {
            super::agent_caps::cmd_agent_caps(&agent_id, json)
        }
        AgentCommands::Api {
            agent_id,
            json,
            manifest,
            rotate_token,
        } => super::agent_api::cmd_agent_api(&agent_id, json, manifest, rotate_token),
        AgentCommands::Chat { agent_id } => super::agent::cmd_agent_chat(config, &agent_id),
        AgentCommands::Kill { agent_id } => super::agent::cmd_agent_kill(config, &agent_id),
        AgentCommands::Set {
            agent_id,
            field,
            value,
        } => super::agent::cmd_agent_set(&agent_id, &field, &value),
    }
}

fn dispatch_workflow_command(sub: WorkflowCommands) {
    match sub {
        WorkflowCommands::List => super::workflow::cmd_workflow_list(),
        WorkflowCommands::Create { file } => super::workflow::cmd_workflow_create(file),
        WorkflowCommands::Get { workflow_id } => super::workflow::cmd_workflow_get(&workflow_id),
        WorkflowCommands::Update { workflow_id, file } => {
            super::workflow::cmd_workflow_update(&workflow_id, file)
        }
        WorkflowCommands::Delete { workflow_id } => {
            super::workflow::cmd_workflow_delete(&workflow_id)
        }
        WorkflowCommands::Run { workflow_id, input } => {
            super::workflow::cmd_workflow_run(&workflow_id, &input)
        }
    }
}

fn dispatch_trigger_command(sub: TriggerCommands) {
    match sub {
        TriggerCommands::List { agent_id } => super::trigger::cmd_trigger_list(agent_id.as_deref()),
        TriggerCommands::Create {
            agent_id,
            pattern_json,
            prompt,
            max_fires,
        } => super::trigger::cmd_trigger_create(&agent_id, &pattern_json, &prompt, max_fires),
        TriggerCommands::Delete { trigger_id } => super::trigger::cmd_trigger_delete(&trigger_id),
    }
}

fn dispatch_approvals_command(sub: ApprovalsCommands) {
    match sub {
        ApprovalsCommands::List { json } => super::approvals::cmd_approvals_list(json),
        ApprovalsCommands::Approve { id } => super::approvals::cmd_approvals_respond(&id, true),
        ApprovalsCommands::Reject { id } => super::approvals::cmd_approvals_respond(&id, false),
    }
}

fn dispatch_cron_command(sub: CronCommands) {
    match sub {
        CronCommands::List { json } => super::cron::cmd_cron_list(json),
        CronCommands::Create {
            agent,
            spec,
            prompt,
            name,
        } => super::cron::cmd_cron_create(&agent, &spec, &prompt, name.as_deref()),
        CronCommands::Delete { id } => super::cron::cmd_cron_delete(&id),
        CronCommands::Enable { id } => super::cron::cmd_cron_toggle(&id, true),
        CronCommands::Disable { id } => super::cron::cmd_cron_toggle(&id, false),
    }
}

fn dispatch_autonomy_command(sub: AutonomyCommands) {
    match sub {
        AutonomyCommands::Status { json, lines, since } => {
            super::autonomy::cmd_autonomy_status(json, lines, since.as_deref())
        }
    }
}

fn dispatch_gateway_command(config: Option<PathBuf>, sub: GatewayCommands) {
    match sub {
        GatewayCommands::Start => super::daemon::cmd_start(config, false),
        GatewayCommands::Stop => super::daemon::cmd_stop(),
        GatewayCommands::Status { json, verbose } => {
            super::status::cmd_status(config, json, verbose)
        }
    }
}

fn dispatch_service_command(sub: ServiceCommands) {
    match sub {
        ServiceCommands::Install {
            manager,
            force,
            dry_run,
            start,
        } => super::service::cmd_service_install(manager, force, dry_run, start),
        ServiceCommands::Start { manager } => super::service::cmd_service_start(manager),
        ServiceCommands::Stop { manager } => super::service::cmd_service_stop(manager),
        ServiceCommands::Restart { manager } => super::service::cmd_service_restart(manager),
        ServiceCommands::Status { json } => super::service::cmd_service_status(json),
        ServiceCommands::Logs { lines, follow } => super::service::cmd_service_logs(lines, follow),
    }
}

fn dispatch_process_command(sub: ProcessCommands) {
    match sub {
        ProcessCommands::List { json } => super::process::cmd_process_list(json),
        ProcessCommands::Kill { process_id } => super::process::cmd_process_kill(&process_id),
    }
}

fn dispatch_webhooks_command(sub: WebhooksCommands) {
    match sub {
        WebhooksCommands::List { json } => super::webhooks::cmd_webhooks_list(json),
        WebhooksCommands::Create { agent, url } => {
            super::webhooks::cmd_webhooks_create(&agent, &url)
        }
        WebhooksCommands::Delete { id } => super::webhooks::cmd_webhooks_delete(&id),
        WebhooksCommands::Test { id } => super::webhooks::cmd_webhooks_test(&id),
    }
}

fn dispatch_capability_command(command: Commands) {
    match command {
        Commands::Skill(sub) => dispatch_skill_command(sub),
        Commands::Channel(sub) => dispatch_channel_command(sub),
        Commands::Hand(sub) => dispatch_hand_command(sub),
        Commands::Integration(sub) => dispatch_integration_command(sub),
        Commands::Voice(sub) => dispatch_voice_command(sub),
        Commands::Embeddings(sub) => dispatch_embeddings_command(sub),
        Commands::Ssh(sub) => dispatch_ssh_command(sub),
        Commands::Config(sub) => dispatch_config_command(sub),
        Commands::Add { name, key } => {
            super::integrations::cmd_integration_add(&name, key.as_deref())
        }
        Commands::Remove { name } => super::integrations::cmd_integration_remove(&name),
        Commands::Integrations { query, doc, out } => {
            dispatch_integrations_command(query, doc, out)
        }
        Commands::Vault(sub) => dispatch_vault_command(sub),
        Commands::New { kind } => super::scaffold::cmd_scaffold(kind),
        Commands::Models(sub) => dispatch_models_command(sub),
        Commands::Security(sub) => dispatch_security_command(sub),
        Commands::Memory(sub) => dispatch_memory_command(sub),
        Commands::Devices(sub) => dispatch_devices_command(sub),
        Commands::Qr => super::devices::cmd_devices_pair(),
        _ => unreachable!("capability command routed to wrong dispatch family"),
    }
}

fn dispatch_skill_command(sub: SkillCommands) {
    match sub {
        SkillCommands::Install { source } => super::skill::cmd_skill_install(&source),
        SkillCommands::List => super::skill::cmd_skill_list(),
        SkillCommands::Doc { out } => super::skill::cmd_skill_doc(out),
        SkillCommands::Remove { name } => super::skill::cmd_skill_remove(&name),
        SkillCommands::Search { query } => super::skill::cmd_skill_search(&query),
        SkillCommands::Create => super::skill::cmd_skill_create(),
    }
}

fn dispatch_channel_command(sub: ChannelCommands) {
    match sub {
        ChannelCommands::List => super::channel::cmd_channel_list(),
        ChannelCommands::Setup { channel } => super::channel::cmd_channel_setup(channel.as_deref()),
        ChannelCommands::Test { channel } => super::channel::cmd_channel_test(&channel),
        ChannelCommands::Enable { channel } => super::channel::cmd_channel_toggle(&channel, true),
        ChannelCommands::Disable { channel } => super::channel::cmd_channel_toggle(&channel, false),
        ChannelCommands::Inbound { command } => dispatch_channel_inbound_command(command),
    }
}

fn dispatch_channel_inbound_command(command: ChannelInboundCommands) {
    match command {
        ChannelInboundCommands::DeadLetters { command } => match command {
            ChannelDeadLetterCommands::Clear { channel } => {
                super::channel_inbound::cmd_clear_inbound_dead_letters(channel.as_deref())
            }
        },
    }
}

fn dispatch_hand_command(sub: HandCommands) {
    match sub {
        HandCommands::List => super::hand::cmd_hand_list(),
        HandCommands::Active => super::hand::cmd_hand_active(),
        HandCommands::Install { path } => super::hand::cmd_hand_install(&path),
        HandCommands::Activate { id } => super::hand::cmd_hand_activate(&id),
        HandCommands::Deactivate { id } => super::hand::cmd_hand_deactivate(&id),
        HandCommands::Info { id } => super::hand::cmd_hand_info(&id),
        HandCommands::CheckDeps { id } => super::hand::cmd_hand_check_deps(&id),
        HandCommands::InstallDeps { id } => super::hand::cmd_hand_install_deps(&id),
        HandCommands::Pause { id } => super::hand::cmd_hand_pause(&id),
        HandCommands::Resume { id } => super::hand::cmd_hand_resume(&id),
    }
}

fn dispatch_integration_command(sub: IntegrationCommands) {
    match sub {
        IntegrationCommands::Setup { name, no_test } => {
            super::integration::cmd_integration_setup_native(&name, !no_test)
        }
        IntegrationCommands::List => super::integration::cmd_integration_setup_list(),
    }
}

fn dispatch_voice_command(sub: VoiceCommands) {
    match sub {
        VoiceCommands::Status { json } => super::voice::cmd_voice_status(json),
        VoiceCommands::Install { best_effort, force } => {
            super::voice::cmd_voice_install(best_effort, force)
        }
        VoiceCommands::Test { json } => super::voice::cmd_voice_test(json),
        VoiceCommands::Doctor { json } => super::voice::cmd_voice_doctor(json),
        VoiceCommands::Uninstall { confirm } => super::voice::cmd_voice_uninstall(confirm),
    }
}

fn dispatch_embeddings_command(sub: EmbeddingsCommands) {
    match sub {
        EmbeddingsCommands::Status { json } => super::embeddings::cmd_embeddings_status(json),
        EmbeddingsCommands::Install { best_effort, force } => {
            super::embeddings::cmd_embeddings_install(best_effort, force)
        }
        EmbeddingsCommands::Doctor { json } => super::embeddings::cmd_embeddings_doctor(json),
    }
}

fn dispatch_ssh_command(sub: SshCommands) {
    match sub {
        SshCommands::Add { name } => super::ssh::cmd_ssh_add(&name),
        SshCommands::List => super::ssh::cmd_ssh_list(),
        SshCommands::Test { name } => super::ssh::cmd_ssh_test(&name),
        SshCommands::Remove { name } => super::ssh::cmd_ssh_remove(&name),
        SshCommands::Use { name } => super::ssh::cmd_ssh_use(&name),
        SshCommands::KnownHosts(kh) => match kh {
            KnownHostsCommands::List => super::ssh::cmd_ssh_kh_list(),
            KnownHostsCommands::Clear => super::ssh::cmd_ssh_kh_clear(),
            KnownHostsCommands::Mode { mode } => super::ssh::cmd_ssh_kh_mode(mode.as_deref()),
        },
    }
}

fn dispatch_config_command(sub: ConfigCommands) {
    match sub {
        ConfigCommands::Show => super::config::cmd_config_show(),
        ConfigCommands::Edit => super::config::cmd_config_edit(),
        ConfigCommands::Get { key } => super::config::cmd_config_get(&key),
        ConfigCommands::Set { key, value } => super::config::cmd_config_set(&key, &value),
        ConfigCommands::Unset { key } => super::config::cmd_config_unset(&key),
        ConfigCommands::SetKey { provider } => super::config::cmd_config_set_key(&provider),
        ConfigCommands::DeleteKey { provider } => super::config::cmd_config_delete_key(&provider),
        ConfigCommands::TestKey { provider } => super::config::cmd_config_test_key(&provider),
        ConfigCommands::InitFull { force } => super::config::cmd_config_init_full(force),
        ConfigCommands::Doctor => super::config::cmd_config_doctor(),
        ConfigCommands::Schema => super::config::cmd_config_schema(),
        ConfigCommands::Reconcile => super::config::cmd_config_reconcile(),
        ConfigCommands::Workspace => super::config::cmd_config_workspace(),
    }
}

fn dispatch_integrations_command(query: Option<String>, doc: bool, out: Option<PathBuf>) {
    if doc {
        super::integrations::cmd_integrations_doc(out);
    } else {
        super::integrations::cmd_integrations_list(query.as_deref());
    }
}

fn dispatch_vault_command(sub: VaultCommands) {
    match sub {
        VaultCommands::Init => super::vault::cmd_vault_init(),
        VaultCommands::Set { key } => super::vault::cmd_vault_set(&key),
        VaultCommands::List => super::vault::cmd_vault_list(),
        VaultCommands::Remove { key } => super::vault::cmd_vault_remove(&key),
    }
}

fn dispatch_models_command(sub: ModelsCommands) {
    match sub {
        ModelsCommands::Current { json } => super::models::cmd_models_current(json),
        ModelsCommands::List { provider, json } => {
            super::models::cmd_models_list(provider.as_deref(), json)
        }
        ModelsCommands::Aliases { json } => super::models::cmd_models_aliases(json),
        ModelsCommands::Providers { json } => super::models::cmd_models_providers(json),
        ModelsCommands::Set { model } => super::models::cmd_models_set(model),
        ModelsCommands::Test { provider, json } => {
            super::models::cmd_models_test(provider.as_deref(), json)
        }
    }
}

fn dispatch_security_command(sub: SecurityCommands) {
    match sub {
        SecurityCommands::Status { json } => super::security::cmd_security_status(json),
        SecurityCommands::Audit { limit, json } => super::security::cmd_security_audit(limit, json),
        SecurityCommands::Verify => super::security::cmd_security_verify(),
    }
}

fn dispatch_memory_command(sub: MemoryCommands) {
    match sub {
        MemoryCommands::List { agent, json } => super::memory::cmd_memory_list(&agent, json),
        MemoryCommands::Get { agent, key, json } => {
            super::memory::cmd_memory_get(&agent, &key, json)
        }
        MemoryCommands::Set { agent, key, value } => {
            super::memory::cmd_memory_set(&agent, &key, &value)
        }
        MemoryCommands::Delete { agent, key } => super::memory::cmd_memory_delete(&agent, &key),
    }
}

fn dispatch_devices_command(sub: DevicesCommands) {
    match sub {
        DevicesCommands::List { json } => super::devices::cmd_devices_list(json),
        DevicesCommands::Pair => super::devices::cmd_devices_pair(),
        DevicesCommands::Remove { id } => super::devices::cmd_devices_remove(&id),
    }
}

fn dispatch_lifecycle_command(command: Commands) {
    match command {
        Commands::Snapshot(sub) => match sub {
            SnapshotCommands::Create { reason, json } => {
                crate::snapshot::cmd_snapshot_create(reason.as_deref(), json);
            }
            SnapshotCommands::List { json } => crate::snapshot::cmd_snapshot_list(json),
            SnapshotCommands::Restore { id, confirm } => {
                crate::snapshot::cmd_snapshot_restore(&id, confirm)
            }
            SnapshotCommands::Prune {
                keep,
                dry_run,
                confirm,
            } => crate::snapshot::cmd_snapshot_prune(keep, dry_run, confirm),
        },
        Commands::Reset {
            confirm,
            factory,
            no_snapshot,
            preserve_secrets,
            preserve_snapshots,
        } => crate::snapshot::cmd_reset(
            confirm,
            factory,
            no_snapshot,
            preserve_secrets,
            preserve_snapshots,
        ),
        Commands::Uninstall {
            confirm,
            keep_config,
        } => super::uninstall::cmd_uninstall(confirm, keep_config),
        _ => unreachable!("lifecycle command routed to wrong dispatch family"),
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
