use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeReason {
    Browse,
    Upload,
    QuickScan,
    Watcher,
    Delete,
    Rename,
    Write,
    Manual,
}

#[derive(Debug, Clone)]
pub struct ReadyIndex {
    pub path: PathBuf,
    pub forced: bool,
    pub reason: ChangeReason,
}

#[derive(Debug, Clone)]
struct PendingIndex {
    first_dirty_at: Instant,
    last_dirty_at: Instant,
    force_at: Instant,
    reason: ChangeReason,
    failures: u32,
    retry_at: Instant,
}

#[derive(Debug, Default)]
struct GateInner {
    last_change: HashMap<PathBuf, Instant>,
    pending: HashMap<PathBuf, PendingIndex>,
}

#[derive(Debug, Default)]
pub struct ChangeGate {
    inner: Mutex<GateInner>,
}

fn normalize(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        PathBuf::from(&*text)
    } else {
        PathBuf::from(trimmed)
    }
}

impl ChangeGate {
    pub fn note_change(
        &self,
        path: &Path,
        reason: ChangeReason,
        _settle: Duration,
        max_defer: Duration,
        max_pending: usize,
    ) {
        let path = normalize(path);
        if path.as_os_str().is_empty() {
            return;
        }
        let now = Instant::now();
        let mut inner = self.inner.lock();
        inner.last_change.insert(path.clone(), now);
        if inner.last_change.len() > 20_000 {
            let cutoff = now.checked_sub(Duration::from_secs(24 * 3600));
            inner
                .last_change
                .retain(|_, at| cutoff.is_none_or(|cutoff| *at >= cutoff));
            if inner.last_change.len() > 20_000 {
                inner.last_change.clear();
            }
        }
        if let Some(pending) = inner.pending.get_mut(&path) {
            pending.last_dirty_at = now;
            pending.reason = reason;
            if matches!(reason, ChangeReason::Manual | ChangeReason::Upload) {
                pending.failures = 0;
                pending.retry_at = now;
            }
            return;
        }
        inner.pending.insert(
            path,
            PendingIndex {
                first_dirty_at: now,
                last_dirty_at: now,
                force_at: now + max_defer.max(Duration::from_millis(1)),
                reason,
                failures: 0,
                retry_at: now,
            },
        );
        if inner.pending.len() > max_pending.max(1) {
            let oldest = inner
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.last_dirty_at)
                .map(|(path, _)| path.clone());
            if let Some(oldest) = oldest {
                inner.pending.remove(&oldest);
            }
        }
    }

    pub fn allow_index(&self, path: &Path, settle: Duration) -> bool {
        let path = normalize(path);
        let now = Instant::now();
        let inner = self.inner.lock();
        let Some(pending) = inner.pending.get(&path) else {
            return true;
        };
        if now < pending.retry_at || pending.failures >= 3 {
            return false;
        }
        now.duration_since(pending.last_dirty_at) >= settle || now >= pending.force_at
    }

    pub fn take_ready(&self, settle: Duration, limit: usize) -> Vec<ReadyIndex> {
        let now = Instant::now();
        let mut inner = self.inner.lock();
        let mut ready = Vec::new();
        for (path, pending) in inner.pending.iter() {
            if ready.len() >= limit.max(1) {
                break;
            }
            if now < pending.retry_at || pending.failures >= 3 {
                continue;
            }
            let forced = now >= pending.force_at;
            if forced || now.duration_since(pending.last_dirty_at) >= settle {
                ready.push(ReadyIndex {
                    path: path.clone(),
                    forced,
                    reason: pending.reason,
                });
            }
        }
        for item in &ready {
            inner.pending.remove(&item.path);
        }
        ready
    }

    pub fn mark_done(&self, path: &Path) {
        let path = normalize(path);
        self.inner.lock().pending.remove(&path);
    }

    pub fn mark_failed(&self, path: &Path, backoff: Duration) {
        let path = normalize(path);
        let now = Instant::now();
        let mut inner = self.inner.lock();
        if let Some(pending) = inner.pending.get_mut(&path) {
            pending.failures = pending.failures.saturating_add(1);
            let multiplier = 1_u32 << pending.failures.min(4);
            pending.retry_at = now + backoff.saturating_mul(multiplier);
            // Keep the path dirty; the next successful scan clears it.
            pending.last_dirty_at = now;
        } else {
            inner.pending.insert(
                path,
                PendingIndex {
                    first_dirty_at: now,
                    last_dirty_at: now,
                    force_at: now + backoff.saturating_mul(4),
                    reason: ChangeReason::Write,
                    failures: 1,
                    retry_at: now + backoff,
                },
            );
        }
    }

    pub fn clear_subtree(&self, prefix: &Path) {
        let prefix = normalize(prefix);
        let mut inner = self.inner.lock();
        inner
            .pending
            .retain(|path, _| !crate::platform::path_within(path, &prefix));
        inner
            .last_change
            .retain(|path, _| !crate::platform::path_within(path, &prefix));
    }

    pub fn pending_len(&self) -> usize {
        self.inner.lock().pending.len()
    }

    pub fn is_pending(&self, path: &Path) -> bool {
        let path = normalize(path);
        self.inner.lock().pending.contains_key(&path)
    }

    pub fn oldest_pending_age(&self) -> Option<Duration> {
        let inner = self.inner.lock();
        inner
            .pending
            .values()
            .map(|pending| pending.first_dirty_at.elapsed())
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle() -> Duration {
        Duration::from_millis(20)
    }

    #[test]
    fn settles_once_and_deduplicates() {
        let gate = ChangeGate::default();
        let path = Path::new("/tmp/gate");
        gate.note_change(
            path,
            ChangeReason::Browse,
            settle(),
            Duration::from_secs(10),
            10,
        );
        assert!(!gate.allow_index(path, settle()));
        std::thread::sleep(Duration::from_millis(30));
        assert!(gate.allow_index(path, settle()));
        let ready = gate.take_ready(settle(), 8);
        assert_eq!(ready.len(), 1);
        assert!(gate.take_ready(settle(), 8).is_empty());
    }

    #[test]
    fn repeated_changes_do_not_create_multiple_pending() {
        let gate = ChangeGate::default();
        let path = Path::new("/tmp/gate");
        for _ in 0..5 {
            gate.note_change(
                path,
                ChangeReason::Upload,
                settle(),
                Duration::from_secs(10),
                10,
            );
        }
        assert_eq!(gate.pending_len(), 1);
    }

    #[test]
    fn max_defer_forces_partial() {
        let gate = ChangeGate::default();
        let path = Path::new("/tmp/gate");
        gate.note_change(
            path,
            ChangeReason::Upload,
            Duration::from_secs(60),
            Duration::from_millis(1),
            10,
        );
        std::thread::sleep(Duration::from_millis(5));
        let ready = gate.take_ready(Duration::from_secs(60), 8);
        assert_eq!(ready.len(), 1);
        assert!(ready[0].forced);
    }

    #[test]
    fn clear_subtree_drops_children() {
        let gate = ChangeGate::default();
        gate.note_change(
            Path::new("/tmp/a"),
            ChangeReason::Write,
            settle(),
            Duration::from_secs(10),
            10,
        );
        gate.note_change(
            Path::new("/tmp/a/b"),
            ChangeReason::Write,
            settle(),
            Duration::from_secs(10),
            10,
        );
        gate.note_change(
            Path::new("/tmp/c"),
            ChangeReason::Write,
            settle(),
            Duration::from_secs(10),
            10,
        );
        gate.clear_subtree(Path::new("/tmp/a"));
        assert_eq!(gate.pending_len(), 1);
    }
}
