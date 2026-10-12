//! Pure, platform-independent logic: filename matching, temp-dir resolution,
//! and the ordered image list. Kept free of UI code so it is unit-testable.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Default pattern. Codex creates pasted images with
/// `tempfile::Builder::new().prefix("codex-clipboard-").suffix(".png")`
/// (codex-rs/tui/src/clipboard_paste.rs, `paste_image_to_temp_png`), which
/// yields names like `codex-clipboard-a1B2c3.png` in `std::env::temp_dir()`.
pub const DEFAULT_PATTERN: &str = "codex-clipboard-*.png";

/// Minimal glob: `*` matches any run of characters, `?` matches one.
/// Matching is case-insensitive on Windows/macOS-style filesystems for safety.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let n: Vec<char> = name.to_lowercase().chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Same directory Rust's `tempfile` crate uses (`std::env::temp_dir()`):
/// macOS `$TMPDIR` (/var/folders/.../T/), Windows `GetTempPath2W` (%TEMP%),
/// Linux `$TMPDIR` or /tmp.
pub fn default_dir() -> PathBuf {
    std::env::temp_dir()
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub number: usize,
}

/// Ordered list of seen images. Numbering follows arrival order like Codex's
/// `[Image #N]` placeholders, and restarts at 1 after `clear()`.
#[derive(Debug)]
pub struct ImageList {
    entries: Vec<Entry>,
    seen: HashSet<PathBuf>,
    last_added: Option<Instant>,
}

impl Default for ImageList {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageList {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            seen: HashSet::new(),
            last_added: None,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Adds a path if never seen. Returns its number when newly added.
    pub fn add(&mut self, path: PathBuf, now: Instant) -> Option<usize> {
        if !self.seen.insert(path.clone()) {
            return None;
        }
        let number = self.entries.len() + 1;
        self.entries.push(Entry { path, number });
        self.last_added = Some(now);
        Some(number)
    }

    /// Clears the visible list. Already-seen files stay ignored so that they
    /// do not reappear on the next scan.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_added = None;
    }

    /// Auto-reset when idle for `idle` since the last new image.
    pub fn should_auto_reset(&self, now: Instant, idle: Option<Duration>) -> bool {
        match (idle, self.last_added) {
            (Some(idle), Some(t)) => !self.entries.is_empty() && now.duration_since(t) >= idle,
            _ => false,
        }
    }
}

/// Lists matching files in `dir` modified at/after `since`, oldest first.
pub fn scan(dir: &Path, pattern: &str, since: SystemTime) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| glob_match(pattern, n))
        })
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            if !md.is_file() || md.len() == 0 {
                return None; // skip files still being created
            }
            let t = md.modified().ok()?;
            (t >= since).then(|| (t, e.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_codex_names() {
        assert!(glob_match(DEFAULT_PATTERN, "codex-clipboard-Ab3xYz.png"));
        assert!(glob_match(DEFAULT_PATTERN, "CODEX-CLIPBOARD-x.PNG"));
        assert!(!glob_match(DEFAULT_PATTERN, "codex-clipboard-x.jpg"));
        assert!(!glob_match(DEFAULT_PATTERN, "xcodex-clipboard-1.png"));
        assert!(!glob_match(DEFAULT_PATTERN, "codex-clipboard-1.png.tmp"));
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*.png", "x.png"));
    }

    #[test]
    fn numbering_and_clear() {
        let mut l = ImageList::new();
        let t = Instant::now();
        assert_eq!(l.add("a".into(), t), Some(1));
        assert_eq!(l.add("b".into(), t), Some(2));
        assert_eq!(l.add("a".into(), t), None);
        l.clear();
        assert!(l.entries().is_empty());
        assert_eq!(l.add("a".into(), t), None, "seen files stay hidden");
        assert_eq!(l.add("c".into(), t), Some(1));
    }

    #[test]
    fn auto_reset() {
        let mut l = ImageList::new();
        let t = Instant::now();
        assert!(!l.should_auto_reset(t, Some(Duration::from_secs(1))));
        l.add("a".into(), t);
        assert!(!l.should_auto_reset(t, Some(Duration::from_secs(1))));
        assert!(l.should_auto_reset(t + Duration::from_secs(2), Some(Duration::from_secs(1))));
        assert!(!l.should_auto_reset(t + Duration::from_secs(2), None));
    }

    #[test]
    fn scan_filters_and_orders() {
        let dir = std::env::temp_dir().join(format!("codex-peek-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let since = SystemTime::now() - Duration::from_secs(5);
        std::fs::write(dir.join("codex-clipboard-1.png"), b"x").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("codex-clipboard-2.png"), b"x").unwrap();
        std::fs::write(dir.join("other.png"), b"x").unwrap();
        std::fs::write(dir.join("codex-clipboard-empty.png"), b"").unwrap();
        let got: Vec<String> = scan(&dir, DEFAULT_PATTERN, since)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(got, vec!["codex-clipboard-1.png", "codex-clipboard-2.png"]);
        assert!(scan(
            &dir,
            DEFAULT_PATTERN,
            SystemTime::now() + Duration::from_secs(60)
        )
        .is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
