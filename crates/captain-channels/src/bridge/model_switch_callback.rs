//! Telegram callback parsing for channel model-switch confirmations.

pub(super) struct ModelSwitchCallbackSelection<'a> {
    pub(super) plan_id: &'a str,
    pub(super) choice: &'a str,
}

impl ModelSwitchCallbackSelection<'_> {
    pub(super) fn is_cancel(&self) -> bool {
        self.choice == "cancel"
    }
}

pub(super) fn parse_model_switch_callback(
    args: &[String],
) -> Option<ModelSwitchCallbackSelection<'_>> {
    let [plan_id, choice] = args else {
        return None;
    };
    Some(ModelSwitchCallbackSelection {
        plan_id: plan_id.as_str(),
        choice: choice.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_plan_and_choice() {
        assert!(parse_model_switch_callback(&[]).is_none());
        assert!(parse_model_switch_callback(&["plan-1".to_string()]).is_none());
        assert!(parse_model_switch_callback(&[
            "plan-1".to_string(),
            "new_session".to_string(),
            "extra".to_string(),
        ])
        .is_none());
    }

    #[test]
    fn callback_preserves_strategy_choice() {
        let args = ["plan-1".to_string(), "compact_session".to_string()];
        let selection = parse_model_switch_callback(&args).expect("valid callback");
        assert_eq!(selection.plan_id, "plan-1");
        assert_eq!(selection.choice, "compact_session");
        assert!(!selection.is_cancel());
    }

    #[test]
    fn callback_detects_cancel_choice() {
        let args = ["plan-1".to_string(), "cancel".to_string()];
        let selection = parse_model_switch_callback(&args).expect("valid callback");
        assert_eq!(selection.plan_id, "plan-1");
        assert!(selection.is_cancel());
    }
}
