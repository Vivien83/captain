use super::*;

#[test]
fn command_family_routes_dispatch_groups() {
    let cases = [
        (Commands::Tui, DispatchFamily::Core),
        (
            Commands::Chat {
                agent: None,
                plain: false,
            },
            DispatchFamily::Core,
        ),
        (
            Commands::Agent(AgentCommands::New { template: None }),
            DispatchFamily::Work,
        ),
        (
            Commands::Gateway(GatewayCommands::Stop),
            DispatchFamily::Work,
        ),
        (
            Commands::Workflow(WorkflowCommands::List),
            DispatchFamily::Work,
        ),
        (
            Commands::Trigger(TriggerCommands::Delete {
                trigger_id: "trigger-1".to_string(),
            }),
            DispatchFamily::Work,
        ),
        (
            Commands::Cron(CronCommands::List { json: false }),
            DispatchFamily::Work,
        ),
        (
            Commands::Skill(SkillCommands::List),
            DispatchFamily::Capability,
        ),
        (
            Commands::Config(ConfigCommands::Show),
            DispatchFamily::Capability,
        ),
        (
            Commands::Models(ModelsCommands::Providers { json: false }),
            DispatchFamily::Capability,
        ),
        (
            Commands::Memory(MemoryCommands::List {
                agent: "captain".to_string(),
                json: false,
            }),
            DispatchFamily::Capability,
        ),
        (Commands::Qr, DispatchFamily::Capability),
        (
            Commands::Snapshot(SnapshotCommands::List { json: false }),
            DispatchFamily::Lifecycle,
        ),
    ];

    for (command, expected) in cases {
        assert_eq!(command_family(&command), expected);
    }
}
