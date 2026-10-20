//! Platform-independent transaction boundary for applying engine updates.

use crate::engine::CompositionUpdate;

/// Host operations used to mirror an engine composition.
pub trait CompositionHost {
    /// Error returned when the host cannot apply an operation.
    type Error;

    /// Starts a new composition with the supplied preedit text.
    fn begin(&mut self, preedit: &str) -> Result<(), Self::Error>;

    /// Replaces the preedit text of the active composition.
    fn replace(&mut self, preedit: &str) -> Result<(), Self::Error>;

    /// Commits text to the host document and ends the composition.
    fn commit(&mut self, text: &str) -> Result<(), Self::Error>;

    /// Cancels the active composition without committing text.
    fn cancel(&mut self) -> Result<(), Self::Error>;
}

/// Tracks whether a host composition is active while applying engine updates.
///
/// Active state changes only after the corresponding host operation succeeds.
/// If a host operation fails, the previous state is retained so the caller can
/// retry the same update.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompositionController {
    active: bool,
}

impl CompositionController {
    /// Creates an inactive controller.
    pub const fn new() -> Self {
        Self { active: false }
    }

    /// Reports whether the host currently has an active composition.
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Applies one engine update to the host.
    ///
    /// Unhandled updates are ignored. A commit takes precedence over a
    /// cancellation or preedit change if an update contains more than one.
    pub fn apply_update<H>(
        &mut self,
        host: &mut H,
        update: &CompositionUpdate,
    ) -> Result<(), H::Error>
    where
        H: CompositionHost + ?Sized,
    {
        if !update.handled {
            return Ok(());
        }

        if let Some(text) = update.committed_text() {
            host.commit(text)?;
            self.active = false;
            return Ok(());
        }

        if update.cancelled {
            return self.cancel_if_active(host);
        }

        if update.preedit.is_empty() {
            return self.cancel_if_active(host);
        }

        if self.active {
            host.replace(&update.preedit)?;
        } else {
            host.begin(&update.preedit)?;
            self.active = true;
        }

        Ok(())
    }

    fn cancel_if_active<H>(&mut self, host: &mut H) -> Result<(), H::Error>
    where
        H: CompositionHost + ?Sized,
    {
        if !self.active {
            return Ok(());
        }

        host.cancel()?;
        self.active = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{CompositionController, CompositionHost};
    use crate::engine::CompositionUpdate;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum HostEvent {
        Begin(String),
        Replace(String),
        Commit(String),
        Cancel,
    }

    impl HostEvent {
        fn call(&self) -> HostCall {
            match self {
                Self::Begin(_) => HostCall::Begin,
                Self::Replace(_) => HostCall::Replace,
                Self::Commit(_) => HostCall::Commit,
                Self::Cancel => HostCall::Cancel,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HostCall {
        Begin,
        Replace,
        Commit,
        Cancel,
    }

    #[derive(Debug, Default)]
    struct FakeHost {
        events: Vec<HostEvent>,
        fail_on: Option<HostCall>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct HostFailure;

    impl FakeHost {
        fn record(&mut self, event: HostEvent) -> Result<(), HostFailure> {
            let call = event.call();
            self.events.push(event);
            if self.fail_on == Some(call) {
                Err(HostFailure)
            } else {
                Ok(())
            }
        }
    }

    impl CompositionHost for FakeHost {
        type Error = HostFailure;

        fn begin(&mut self, preedit: &str) -> Result<(), Self::Error> {
            self.record(HostEvent::Begin(preedit.to_owned()))
        }

        fn replace(&mut self, preedit: &str) -> Result<(), Self::Error> {
            self.record(HostEvent::Replace(preedit.to_owned()))
        }

        fn commit(&mut self, text: &str) -> Result<(), Self::Error> {
            self.record(HostEvent::Commit(text.to_owned()))
        }

        fn cancel(&mut self) -> Result<(), Self::Error> {
            self.record(HostEvent::Cancel)
        }
    }

    fn preedit_update(preedit: &str) -> CompositionUpdate {
        CompositionUpdate {
            handled: true,
            preedit: preedit.to_owned(),
            ..CompositionUpdate::default()
        }
    }

    fn commit_update(text: &str) -> CompositionUpdate {
        CompositionUpdate {
            handled: true,
            commit: Some(text.to_owned()),
            ..CompositionUpdate::default()
        }
    }

    fn cancel_update() -> CompositionUpdate {
        CompositionUpdate {
            handled: true,
            cancelled: true,
            ..CompositionUpdate::default()
        }
    }

    #[test]
    fn first_preedit_begins_and_later_preedit_replaces() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("n"))
            .expect("begin composition");
        assert!(controller.is_active());

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("replace preedit");
        assert!(controller.is_active());

        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("n".to_owned()),
                HostEvent::Replace("ni".to_owned()),
            ]
        );
    }

    #[test]
    fn unhandled_update_does_not_call_host_or_change_active_state() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();
        let mut update = preedit_update("ignored");
        update.handled = false;

        controller
            .apply_update(&mut host, &update)
            .expect("ignore unhandled update");
        assert!(!controller.is_active());
        assert!(host.events.is_empty());

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        let events_while_active = host.events.clone();

        controller
            .apply_update(&mut host, &update)
            .expect("ignore unhandled update while active");
        assert!(controller.is_active());
        assert_eq!(host.events, events_while_active);
    }

    #[test]
    fn commit_ends_composition_and_next_input_begins_again() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        controller
            .apply_update(&mut host, &commit_update("你"))
            .expect("commit composition");
        assert!(!controller.is_active());

        controller
            .apply_update(&mut host, &preedit_update("h"))
            .expect("begin next composition");
        assert!(controller.is_active());

        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("ni".to_owned()),
                HostEvent::Commit("你".to_owned()),
                HostEvent::Begin("h".to_owned()),
            ]
        );
    }

    #[test]
    fn cancellation_ends_composition_and_next_input_begins_again() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        controller
            .apply_update(&mut host, &cancel_update())
            .expect("cancel composition");
        assert!(!controller.is_active());

        controller
            .apply_update(&mut host, &preedit_update("h"))
            .expect("begin next composition");
        assert!(controller.is_active());

        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("ni".to_owned()),
                HostEvent::Cancel,
                HostEvent::Begin("h".to_owned()),
            ]
        );
    }

    #[test]
    fn handled_empty_preedit_cancels_active_composition() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        controller
            .apply_update(&mut host, &preedit_update(""))
            .expect("clear composition");

        assert!(!controller.is_active());
        assert_eq!(
            host.events,
            vec![HostEvent::Begin("ni".to_owned()), HostEvent::Cancel]
        );
    }

    #[test]
    fn inactive_cancellation_does_not_call_host() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &cancel_update())
            .expect("ignore inactive cancellation");

        assert!(!controller.is_active());
        assert!(host.events.is_empty());
    }

    #[test]
    fn failed_begin_leaves_controller_inactive() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost {
            fail_on: Some(HostCall::Begin),
            ..FakeHost::default()
        };

        assert_eq!(
            controller.apply_update(&mut host, &preedit_update("n")),
            Err(HostFailure)
        );
        assert!(!controller.is_active());
        assert_eq!(host.events, vec![HostEvent::Begin("n".to_owned())]);
    }

    #[test]
    fn failed_replace_keeps_controller_active_for_retry() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("n"))
            .expect("begin composition");
        host.fail_on = Some(HostCall::Replace);

        assert_eq!(
            controller.apply_update(&mut host, &preedit_update("ni")),
            Err(HostFailure)
        );
        assert!(controller.is_active());

        host.fail_on = None;
        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("retry replacement");
        assert!(controller.is_active());
        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("n".to_owned()),
                HostEvent::Replace("ni".to_owned()),
                HostEvent::Replace("ni".to_owned()),
            ]
        );
    }

    #[test]
    fn failed_commit_keeps_controller_active_for_retry() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        host.fail_on = Some(HostCall::Commit);

        assert_eq!(
            controller.apply_update(&mut host, &commit_update("你")),
            Err(HostFailure)
        );
        assert!(controller.is_active());

        host.fail_on = None;
        controller
            .apply_update(&mut host, &commit_update("你"))
            .expect("retry commit");
        assert!(!controller.is_active());
        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("ni".to_owned()),
                HostEvent::Commit("你".to_owned()),
                HostEvent::Commit("你".to_owned()),
            ]
        );
    }

    #[test]
    fn failed_cancel_keeps_controller_active_for_retry() {
        let mut controller = CompositionController::new();
        let mut host = FakeHost::default();

        controller
            .apply_update(&mut host, &preedit_update("ni"))
            .expect("begin composition");
        host.fail_on = Some(HostCall::Cancel);

        assert_eq!(
            controller.apply_update(&mut host, &cancel_update()),
            Err(HostFailure)
        );
        assert!(controller.is_active());

        host.fail_on = None;
        controller
            .apply_update(&mut host, &cancel_update())
            .expect("retry cancellation");
        assert!(!controller.is_active());
        assert_eq!(
            host.events,
            vec![
                HostEvent::Begin("ni".to_owned()),
                HostEvent::Cancel,
                HostEvent::Cancel,
            ]
        );
    }
}
