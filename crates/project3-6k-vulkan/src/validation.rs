use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Debug)]
/// Shared read-only access to validation messages collected by a renderer.
pub struct ValidationMonitor(Arc<ValidationState>);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// Snapshot of validation warning/error counts and the messages received so far.
pub struct ValidationReport {
    /// Number of warning-severity messages.
    pub warnings: usize,
    /// Number of error-severity messages.
    pub errors: usize,
    /// Owned messages in the order they were received.
    pub messages: Vec<String>,
}

impl ValidationReport {
    /// Returns whether the report contains any warning or error.
    pub fn has_issues(&self) -> bool {
        self.warnings != 0 || self.errors != 0
    }
}

impl ValidationMonitor {
    /// Copies the current counts and messages into an owned snapshot.
    pub fn report(&self) -> ValidationReport {
        ValidationReport {
            warnings: self.0.warnings.load(Ordering::Relaxed),
            errors: self.0.errors.load(Ordering::Relaxed),
            messages: self
                .0
                .messages
                .lock()
                .map(|messages| messages.clone())
                .unwrap_or_else(|poisoned| poisoned.into_inner().clone()),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ValidationState {
    warnings: AtomicUsize,
    errors: AtomicUsize,
    messages: Mutex<Vec<String>>,
}

impl ValidationState {
    pub(super) fn monitor(self: &Arc<Self>) -> ValidationMonitor {
        ValidationMonitor(Arc::clone(self))
    }

    pub(super) fn record_warning(&self, message: String) {
        self.warnings.fetch_add(1, Ordering::Relaxed);
        self.push_message("WARNING", message);
    }

    pub(super) fn record_error(&self, message: String) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        self.push_message("ERROR", message);
    }

    fn push_message(&self, severity: &str, message: String) {
        if let Ok(mut messages) = self.messages.lock() {
            messages.push(format!("{severity}: {message}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ValidationState;
    use std::sync::Arc;

    #[test]
    fn monitor_reports_warning_and_error_messages() {
        let state = Arc::new(ValidationState::default());
        let monitor = state.monitor();
        state.record_warning("warning message".into());
        state.record_error("error message".into());

        let report = monitor.report();
        assert_eq!(report.warnings, 1);
        assert_eq!(report.errors, 1);
        assert!(report.has_issues());
        assert_eq!(
            report.messages,
            ["WARNING: warning message", "ERROR: error message"]
        );
    }
}
