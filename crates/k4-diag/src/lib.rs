//! Structured, levelled, bounded diagnostic log (FR-DIAG-01).
//!
//! A `DiagLog` holds the most recent formatted log lines (oldest evicted past
//! capacity), filtered by a minimum level. It is pure and clock-free so it is
//! unit-testable; callers redact secrets before logging (FR-DIAG-03,
//! `k4_config::redact`).

use std::collections::VecDeque;

/// Log severity, ordered `Debug < Info < Warn < Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

/// A bounded ring of formatted log lines.
#[derive(Debug)]
pub struct DiagLog {
    capacity: usize,
    min_level: Level,
    lines: VecDeque<String>,
}

impl DiagLog {
    /// Create a log holding at most `capacity` lines, dropping records below
    /// `min_level`.
    pub fn new(capacity: usize, min_level: Level) -> Self {
        Self {
            capacity: capacity.max(1),
            min_level,
            lines: VecDeque::new(),
        }
    }

    /// Record a line `"[LEVEL] category: message"` if `level >= min_level`.
    pub fn log(&mut self, level: Level, category: &str, message: &str) {
        if level < self.min_level {
            return;
        }
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines
            .push_back(format!("[{}] {category}: {message}", level.label()));
    }

    /// Number of retained lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The most recent `n` lines, oldest first.
    pub fn recent(&self, n: usize) -> Vec<String> {
        let start = self.lines.len().saturating_sub(n);
        self.lines.iter().skip(start).cloned().collect()
    }
}
