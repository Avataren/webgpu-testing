//! Process-wide ring buffer of recent log records for remote clients.
//!
//! The engine's logger forwards every record it emits to [`record`]. Each entry gets a
//! monotonically increasing sequence number, so clients can poll for "everything after
//! the last entry I saw" with [`entries_since`].

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Number of entries kept; older ones are dropped.
const CAPACITY: usize = 5000;

/// One captured log record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    pub level: log::Level,
    pub target: String,
    pub message: String,
}

struct Tap {
    next_seq: u64,
    lines: VecDeque<LogLine>,
}

static TAP: Mutex<Tap> = Mutex::new(Tap {
    next_seq: 1,
    lines: VecDeque::new(),
});

/// Sequence number of the last entry logged before the most recent play session started.
static PLAY_START_SEQ: AtomicU64 = AtomicU64::new(0);

/// Remember that play mode starts now, so clients can ask for the log "since play
/// started" and skip everything an earlier session or the editor logged.
pub fn mark_play_start() {
    PLAY_START_SEQ.store(latest_seq(), Ordering::Relaxed);
}

/// Sequence number to pass as `after` to get only what was logged since the most recent
/// play session started (0 if none has).
pub fn play_start_seq() -> u64 {
    PLAY_START_SEQ.load(Ordering::Relaxed)
}

/// Store a log record.
pub fn record(record: &log::Record<'_>) {
    push(record.level(), record.target(), record.args().to_string());
}

/// Store a message as if it had been logged.
pub fn push(level: log::Level, target: &str, message: String) {
    let timestamp_ms = crate::time::unix_millis();
    let Ok(mut tap) = TAP.lock() else {
        return;
    };
    let seq = tap.next_seq;
    tap.next_seq += 1;
    if tap.lines.len() == CAPACITY {
        tap.lines.pop_front();
    }
    tap.lines.push_back(LogLine {
        seq,
        timestamp_ms,
        level,
        target: target.to_owned(),
        message,
    });
}

/// Entries with a sequence number greater than `after`, oldest first, that are at
/// least as severe as `min_level` and match `filter`.
pub fn entries_since(
    after: u64,
    min_level: log::Level,
    filter: impl Fn(&LogLine) -> bool,
) -> Vec<LogLine> {
    let Ok(tap) = TAP.lock() else {
        return Vec::new();
    };
    tap.lines
        .iter()
        .filter(|line| line.seq > after && line.level <= min_level && filter(line))
        .cloned()
        .collect()
}

/// Sequence number of the most recent entry, or 0 if nothing was logged yet.
pub fn latest_seq() -> u64 {
    TAP.lock().map_or(0, |tap| tap.next_seq - 1)
}

/// A logger that records into the tap and forwards to another logger.
pub struct TeeLogger<L> {
    inner: L,
}

impl<L: log::Log> TeeLogger<L> {
    pub fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<L: log::Log> log::Log for TeeLogger<L> {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.inner.enabled(record.metadata()) {
            self::record(record);
            self.inner.log(record);
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_since_filters_by_sequence_and_level() {
        let before = latest_seq();
        push(log::Level::Info, "log_tap_test", "first".into());
        push(log::Level::Debug, "log_tap_test", "second".into());
        push(log::Level::Error, "log_tap_test", "third".into());

        let mine = |line: &LogLine| line.target == "log_tap_test";
        let all = entries_since(before, log::Level::Trace, mine);
        assert_eq!(all.len(), 3);
        assert!(all.windows(2).all(|pair| pair[0].seq < pair[1].seq));

        let warnings = entries_since(before, log::Level::Warn, mine);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].message, "third");

        let after_first = entries_since(all[0].seq, log::Level::Trace, mine);
        assert_eq!(after_first.len(), 2);
    }
}
