use super::navigation_state::{AutomationView, CapabilitiesView, ConnectionsView, LearningView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HubSlashRoute {
    Automation(AutomationView),
    Learning(LearningView),
    Capabilities(CapabilitiesView),
    Connections(ConnectionsView),
}

pub(crate) fn hub_slash_route_for_command(command: &str) -> Option<HubSlashRoute> {
    match command {
        "/automation" | "/workflows" => Some(HubSlashRoute::Automation(AutomationView::Workflows)),
        "/triggers" => Some(HubSlashRoute::Automation(AutomationView::Triggers)),
        "/cron" | "/scheduler" => Some(HubSlashRoute::Automation(AutomationView::Cron)),
        "/approvals" => Some(HubSlashRoute::Automation(AutomationView::Approvals)),
        "/learning" => Some(HubSlashRoute::Learning(LearningView::Review)),
        "/skills-proposed" | "/proposed" => {
            Some(HubSlashRoute::Learning(LearningView::SkillProposals))
        }
        "/memory" => Some(HubSlashRoute::Learning(LearningView::Memory)),
        "/graph" => Some(HubSlashRoute::Learning(LearningView::Graph)),
        "/skills" | "/capabilities" => Some(HubSlashRoute::Capabilities(CapabilitiesView::Skills)),
        "/channels" => Some(HubSlashRoute::Connections(ConnectionsView::Channels)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "hub_slash_state/tests.rs"]
mod tests;
