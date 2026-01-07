//! Verbose transcript logging for workflow steps.

pub(crate) struct Transcript {
    enabled: bool,
    started: bool,
}

impl Transcript {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            started: false,
        }
    }

    pub(crate) fn note(&mut self, message: impl AsRef<str>) {
        if !self.enabled {
            return;
        }
        self.start();
        eprintln!("- {}", message.as_ref());
    }

    pub(crate) fn block(&mut self, title: &str, content: &str) {
        if !self.enabled {
            return;
        }
        self.start();
        eprintln!("--- {title} ---");
        eprintln!("{content}");
        eprintln!("--- end {title} ---");
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        eprintln!("transcript:");
    }
}
