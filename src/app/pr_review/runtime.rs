//! In-flight PR work. Closing a pane and cancelling a worker are separate actions.

use std::sync::mpsc::{Receiver, TryRecvError};

use super::{InvestigationOutcome, PrReview};

/// One fetch and one investigation slot, preserving the original independent
/// cancellation and receiver-drop behavior. AppMode remains the result target.
pub(crate) struct PrReviewWork {
    fetch: Option<Receiver<anyhow::Result<PrReview>>>,
    investigation: Option<Receiver<InvestigationOutcome>>,
    github: std::sync::Arc<dyn super::github_access::TriageGithub>,
    investigation_runner: super::github_access::InvestigationRunner,
}

impl Default for PrReviewWork {
    fn default() -> Self {
        Self {
            fetch: None,
            investigation: None,
            github: std::sync::Arc::new(super::github_access::GhTriageGithub),
            investigation_runner: crate::headless::HeadlessRunner::run_investigation,
        }
    }
}

impl PrReviewWork {
    pub(crate) fn github(&self) -> std::sync::Arc<dyn super::github_access::TriageGithub> {
        self.github.clone()
    }

    pub(crate) fn investigation_runner(&self) -> super::github_access::InvestigationRunner {
        self.investigation_runner
    }

    #[cfg(test)]
    pub(crate) fn set_github_for_test(
        &mut self,
        github: std::sync::Arc<dyn super::github_access::TriageGithub>,
    ) {
        self.github = github;
    }

    #[cfg(test)]
    pub(crate) fn set_investigation_runner_for_test(
        &mut self,
        runner: super::github_access::InvestigationRunner,
    ) {
        self.investigation_runner = runner;
    }

    pub(crate) fn begin_fetch(&mut self, receiver: Receiver<anyhow::Result<PrReview>>) {
        self.fetch = Some(receiver);
    }

    pub(crate) fn fetch_pending(&self) -> bool {
        self.fetch.is_some()
    }

    pub(crate) fn poll_fetch(&self) -> Option<Result<anyhow::Result<PrReview>, TryRecvError>> {
        self.fetch.as_ref().map(Receiver::try_recv)
    }

    pub(crate) fn cancel_fetch(&mut self) {
        self.fetch = None;
    }

    pub(crate) fn begin_investigation(&mut self, receiver: Receiver<InvestigationOutcome>) {
        self.investigation = Some(receiver);
    }

    pub(crate) fn investigation_pending(&self) -> bool {
        self.investigation.is_some()
    }

    pub(crate) fn poll_investigation(&self) -> Option<Result<InvestigationOutcome, TryRecvError>> {
        self.investigation.as_ref().map(Receiver::try_recv)
    }

    pub(crate) fn cancel_investigation(&mut self) {
        self.investigation = None;
    }
}
