use serde::Serialize;

pub fn authorize_close(
    workspace: &mut crate::workspace::Workspace,
    discard_changes: bool,
) -> crate::document::Result<()> {
    if workspace.has_unsaved_changes() && !discard_changes {
        return Err(crate::document::Error::Invalid(
            "There are unsaved PDF tabs or pending drafts. Review them or explicitly discard all changes before quitting.".into(),
        ));
    }
    workspace.clean_recovery_exit()
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Unavailable,
    #[default]
    Idle,
    Checking,
    Downloading,
    Ready,
    Installing,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: Phase,
    pub version: Option<String>,
    pub received: u64,
    pub total: Option<u64>,
    pub message: Option<String>,
}

pub struct Lifecycle<T> {
    pub status: Status,
    staged: Option<T>,
    closing: bool,
}

impl<T> Default for Lifecycle<T> {
    fn default() -> Self {
        Self {
            status: Status::default(),
            staged: None,
            closing: false,
        }
    }
}

impl<T> Lifecycle<T> {
    pub fn unavailable(&mut self, message: &str) {
        self.status.phase = Phase::Unavailable;
        self.status.message = Some(message.into());
    }

    pub fn begin_check(&mut self) -> bool {
        if self.closing
            || matches!(
                self.status.phase,
                Phase::Unavailable
                    | Phase::Checking
                    | Phase::Downloading
                    | Phase::Ready
                    | Phase::Installing
            )
        {
            return false;
        }
        self.status = Status {
            phase: Phase::Checking,
            ..Default::default()
        };
        true
    }

    pub fn downloading(&mut self, version: String) {
        if !self.closing {
            self.status.phase = Phase::Downloading;
            self.status.version = Some(version);
        }
    }

    pub fn progress(&mut self, received: u64, total: Option<u64>) {
        if !self.closing {
            self.status.received = received;
            self.status.total = total;
        }
    }

    pub fn finish(&mut self, staged: Option<T>) {
        if self.closing {
            return;
        }
        self.status.phase = if staged.is_some() {
            Phase::Ready
        } else {
            Phase::Idle
        };
        if staged.is_none() {
            self.status.message = Some("You're up to date.".into());
        }
        self.staged = staged;
    }

    pub fn fail(&mut self, message: String) {
        if self.closing {
            return;
        }
        self.staged = None;
        self.status.phase = Phase::Error;
        self.status.message = Some(message);
    }

    // Called only after the document/draft/recovery close authority accepts quitting.
    pub fn begin_close(&mut self) -> Option<T> {
        if self.closing {
            return None;
        }
        self.closing = true;
        let staged = self.staged.take();
        if staged.is_some() {
            self.status.phase = Phase::Installing;
        }
        staged
    }

    pub fn close_failed(&mut self, message: String) {
        self.closing = false;
        self.fail(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_never_install_before_an_accepted_close() {
        let mut state = Lifecycle::<&str>::default();
        assert!(state.begin_check());
        assert!(!state.begin_check());
        state.downloading("0.2.0".into());
        state.progress(10, Some(20));
        assert_eq!(state.status.received, 10);
        state.finish(Some("verified bytes"));
        assert_eq!(state.status.phase, Phase::Ready);
        assert!(!state.begin_check());
        assert_eq!(state.begin_close(), Some("verified bytes"));
        assert_eq!(state.status.phase, Phase::Installing);
        assert_eq!(state.begin_close(), None);
    }

    #[test]
    fn closing_during_a_download_does_not_wait_or_accept_a_late_result() {
        let mut state = Lifecycle::<u8>::default();
        state.begin_check();
        state.downloading("0.2.0".into());
        assert_eq!(state.begin_close(), None);
        state.finish(Some(42));
        state.fail("late network failure".into());
        assert!(!state.begin_check());
        assert_eq!(state.begin_close(), None);
    }

    #[test]
    fn failed_install_keeps_app_open_and_next_close_can_skip_update() {
        let mut state = Lifecycle::<u8>::default();
        state.begin_check();
        state.finish(Some(42));
        assert_eq!(state.begin_close(), Some(42));
        state.close_failed("Installation failed; quit again to skip.".into());
        assert_eq!(state.status.phase, Phase::Error);
        assert_eq!(state.begin_close(), None);
    }

    #[test]
    fn offline_errors_retry_without_becoming_ready() {
        let mut state = Lifecycle::<u8>::default();
        state.begin_check();
        state.fail("Offline".into());
        assert_eq!(state.status.phase, Phase::Error);
        assert!(state.begin_check());
        state.finish(None);
        assert_eq!(state.status.phase, Phase::Idle);
        assert_eq!(state.status.message.as_deref(), Some("You're up to date."));
    }

    #[test]
    fn unsupported_installs_never_check_or_stage() {
        let mut state = Lifecycle::<u8>::default();
        state.unavailable("Use the package manager.");
        assert!(!state.begin_check());
        assert_eq!(state.begin_close(), None);
    }
}
