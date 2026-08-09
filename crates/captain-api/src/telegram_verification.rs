use captain_channels::TelegramVerificationPresentation;
use std::time::Duration;
use tokio::time::Instant;

pub(crate) const TELEGRAM_VERIFICATION_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramVerificationState {
    Idle,
    Pending { deadline: Instant },
    Visible(TelegramVerificationPresentation),
    Terminal(TelegramVerificationPresentation),
}

#[derive(Debug, Clone)]
pub(crate) struct TelegramVerificationGate {
    state: TelegramVerificationState,
}

impl Default for TelegramVerificationGate {
    fn default() -> Self {
        Self {
            state: TelegramVerificationState::Idle,
        }
    }
}

impl TelegramVerificationGate {
    pub(crate) fn observe_phase(
        &mut self,
        phase: &str,
        now: Instant,
    ) -> Option<TelegramVerificationPresentation> {
        match phase {
            "verifying" => {
                if matches!(self.state, TelegramVerificationState::Idle) {
                    self.state = TelegramVerificationState::Pending {
                        deadline: now + TELEGRAM_VERIFICATION_DELAY,
                    };
                }
                None
            }
            "correcting" => self.present(TelegramVerificationPresentation::Correcting),
            "verification_verified" => match self.state {
                TelegramVerificationState::Visible(_) => {
                    self.terminate(TelegramVerificationPresentation::Verified)
                }
                TelegramVerificationState::Pending { .. } => {
                    self.state = TelegramVerificationState::Idle;
                    None
                }
                TelegramVerificationState::Idle | TelegramVerificationState::Terminal(_) => None,
            },
            "verification_incomplete" => {
                self.terminate(TelegramVerificationPresentation::Incomplete)
            }
            "done" | "error" => match self.state {
                TelegramVerificationState::Pending { .. } => {
                    self.state = TelegramVerificationState::Idle;
                    None
                }
                TelegramVerificationState::Visible(_) => {
                    self.terminate(TelegramVerificationPresentation::Incomplete)
                }
                TelegramVerificationState::Idle | TelegramVerificationState::Terminal(_) => None,
            },
            _ => None,
        }
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        match self.state {
            TelegramVerificationState::Pending { deadline } => Some(deadline),
            _ => None,
        }
    }

    pub(crate) fn deadline_elapsed(
        &mut self,
        now: Instant,
    ) -> Option<TelegramVerificationPresentation> {
        let TelegramVerificationState::Pending { deadline } = self.state else {
            return None;
        };
        if now < deadline {
            return None;
        }
        self.present(TelegramVerificationPresentation::Verifying)
    }

    fn present(
        &mut self,
        presentation: TelegramVerificationPresentation,
    ) -> Option<TelegramVerificationPresentation> {
        if matches!(
            self.state,
            TelegramVerificationState::Visible(current) if current == presentation
        ) || matches!(self.state, TelegramVerificationState::Terminal(_))
        {
            return None;
        }
        self.state = TelegramVerificationState::Visible(presentation);
        Some(presentation)
    }

    fn terminate(
        &mut self,
        presentation: TelegramVerificationPresentation,
    ) -> Option<TelegramVerificationPresentation> {
        if matches!(
            self.state,
            TelegramVerificationState::Terminal(current) if current == presentation
        ) {
            return None;
        }
        self.state = TelegramVerificationState::Terminal(presentation);
        Some(presentation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_verification_stays_silent() {
        let start = Instant::now();
        let mut gate = TelegramVerificationGate::default();

        assert_eq!(gate.observe_phase("verifying", start), None);
        assert_eq!(
            gate.observe_phase("verification_verified", start + Duration::from_secs(1)),
            None
        );
        assert_eq!(gate.deadline(), None);
    }

    #[test]
    fn delayed_verification_updates_one_visible_card() {
        let start = Instant::now();
        let mut gate = TelegramVerificationGate::default();
        gate.observe_phase("verifying", start);

        assert_eq!(
            gate.deadline_elapsed(start + TELEGRAM_VERIFICATION_DELAY),
            Some(TelegramVerificationPresentation::Verifying)
        );
        assert_eq!(
            gate.observe_phase("verification_verified", start + Duration::from_secs(4)),
            Some(TelegramVerificationPresentation::Verified)
        );
        assert_eq!(
            gate.observe_phase("verification_verified", start + Duration::from_secs(5)),
            None
        );
    }

    #[test]
    fn correction_and_incomplete_are_immediately_visible() {
        let start = Instant::now();
        let mut gate = TelegramVerificationGate::default();

        assert_eq!(
            gate.observe_phase("correcting", start),
            Some(TelegramVerificationPresentation::Correcting)
        );
        assert_eq!(gate.observe_phase("verifying", start), None);
        assert_eq!(
            gate.observe_phase("verification_incomplete", start),
            Some(TelegramVerificationPresentation::Incomplete)
        );
    }

    #[test]
    fn abnormal_stream_end_never_leaves_a_visible_running_card() {
        let start = Instant::now();
        let mut quick = TelegramVerificationGate::default();
        quick.observe_phase("verifying", start);
        assert_eq!(quick.observe_phase("done", start), None);
        assert_eq!(quick.deadline(), None);

        let mut visible = TelegramVerificationGate::default();
        visible.observe_phase("verifying", start);
        visible.deadline_elapsed(start + TELEGRAM_VERIFICATION_DELAY);
        assert_eq!(
            visible.observe_phase("error", start + TELEGRAM_VERIFICATION_DELAY),
            Some(TelegramVerificationPresentation::Incomplete)
        );
    }
}
