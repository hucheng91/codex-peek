//! Session registry with reference counting. Each Codex session registers
//! with an optional TUI pid; the window closes when the last session ends.

use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct Sessions {
    map: BTreeMap<String, Option<u32>>,
    had_any: bool,
    /// Consecutive checks that found no Codex TUI at all (for pid-less sessions).
    orphan_misses: u32,
}

/// After this many consecutive "no Codex TUI anywhere" checks, pid-less sessions end.
pub const ORPHAN_LIMIT: u32 = 2;

impl Sessions {
    pub fn start(&mut self, id: &str, pid: Option<u32>) {
        self.had_any = true;
        let e = self.map.entry(id.to_string()).or_insert(None);
        if pid.is_some() {
            *e = pid;
        }
    }

    pub fn end(&mut self, id: &str) {
        self.map.remove(id);
    }

    pub fn claimed(&self) -> Vec<u32> {
        self.map.values().flatten().copied().collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn has_pidless(&self) -> bool {
        self.map.values().any(Option::is_none)
    }

    /// Drops sessions whose TUI has exited.
    pub fn reap_dead(&mut self, alive: impl Fn(u32) -> bool) {
        self.map.retain(|_, pid| pid.is_none_or(&alive));
    }

    /// Fallback for sessions without a known pid: end them once no Codex TUI
    /// has been seen for `ORPHAN_LIMIT` consecutive checks.
    pub fn check_orphans(&mut self, any_tui_running: bool) {
        if any_tui_running {
            self.orphan_misses = 0;
            return;
        }
        self.orphan_misses += 1;
        if self.orphan_misses >= ORPHAN_LIMIT {
            self.map.retain(|_, pid| pid.is_some());
            self.orphan_misses = 0;
        }
    }

    /// True once at least one session registered and none remain.
    pub fn all_ended(&self) -> bool {
        self.had_any && self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcount_two_sessions() {
        let mut s = Sessions::default();
        assert!(!s.all_ended());
        s.start("a", Some(10));
        s.start("b", Some(20));
        s.start("a", None); // re-register without pid keeps pid
        assert_eq!(s.claimed(), vec![10, 20]);
        s.reap_dead(|p| p != 10);
        assert_eq!(s.len(), 1);
        assert!(!s.all_ended());
        s.end("b");
        assert!(s.all_ended());
    }

    #[test]
    fn orphan_fallback() {
        let mut s = Sessions::default();
        s.start("x", None);
        s.start("y", Some(5));
        assert!(s.has_pidless());
        s.check_orphans(false);
        assert_eq!(s.len(), 2);
        s.check_orphans(true); // resets
        s.check_orphans(false);
        assert_eq!(s.len(), 2);
        s.check_orphans(false);
        assert_eq!(s.len(), 1);
        assert!(!s.has_pidless());
    }
}
