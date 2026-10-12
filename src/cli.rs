//! Command-line parsing (hand-rolled to keep the binary small).

use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47_821;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Show the window (default).
    Run,
    /// Codex SessionStart hook: register session, launch window if needed.
    HookStart,
    /// Codex SessionEnd hook: unregister session; window exits when none remain.
    HookStop,
    Help,
    Version,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub dir: Option<PathBuf>,
    pub pattern: Option<String>,
    pub idle_reset: Option<Duration>,
    pub include_existing: bool,
    pub port: u16,
    /// Internal: window was started by a hook and should exit when all sessions end.
    pub managed: bool,
}

pub const HELP: &str = "codex-peek - floating preview of images pasted into Codex CLI

USAGE: codex-peek [OPTIONS]

OPTIONS:
  --dir <PATH>        Directory to watch (default: OS temp dir)
  --pattern <GLOB>    Filename glob (default: codex-clipboard-*.png)
  --idle <SECONDS>    Auto-clear after N seconds without new images (0 = off, default)
  --existing          Also show matching files created in the last 10 minutes
  --port <N>          Localhost control port (default 47821)
  --hook-start        Codex SessionStart hook entry point (reads JSON on stdin)
  --hook-stop         Codex SessionEnd hook entry point (reads JSON on stdin)
  -h, --help          Show help
  -V, --version       Show version
";

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
    let mut o = Options {
        mode: Mode::Run,
        dir: None,
        pattern: None,
        idle_reset: None,
        include_existing: false,
        port: DEFAULT_PORT,
        managed: false,
    };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut val = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or(format!("{name} needs a value"))
        };
        match key.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(val("--dir")?)),
            "--pattern" => o.pattern = Some(val("--pattern")?),
            "--idle" => {
                let s: u64 = val("--idle")?
                    .parse()
                    .map_err(|_| "--idle expects seconds")?;
                o.idle_reset = (s > 0).then(|| Duration::from_secs(s));
            }
            "--port" => {
                o.port = val("--port")?
                    .parse()
                    .map_err(|_| "--port expects a number")?
            }
            "--existing" => o.include_existing = true,
            "--managed" => o.managed = true,
            "--hook-start" => o.mode = Mode::HookStart,
            "--hook-stop" => o.mode = Mode::HookStop,
            "-h" | "--help" => o.mode = Mode::Help,
            "-V" | "--version" => o.mode = Mode::Version,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(o)
}

/// Extracts `session_id` from Codex hook stdin JSON.
pub fn session_id_from_hook_input(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| v.get("session_id")?.as_str().map(str::to_owned))
        .filter(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .unwrap_or_else(|| "default".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(a: &[&str]) -> Result<Options, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        let o = p(&["--dir", "/x", "--pattern=a*.png", "--idle", "30"]).unwrap();
        assert_eq!(o.dir, Some(PathBuf::from("/x")));
        assert_eq!(o.pattern.as_deref(), Some("a*.png"));
        assert_eq!(o.idle_reset, Some(Duration::from_secs(30)));
        assert_eq!(o.mode, Mode::Run);
        assert_eq!(p(&["--idle", "0"]).unwrap().idle_reset, None);
        assert_eq!(p(&["--hook-start"]).unwrap().mode, Mode::HookStart);
        assert!(p(&["--bogus"]).is_err());
        assert!(p(&["--dir"]).is_err());
    }

    #[test]
    fn session_id() {
        assert_eq!(
            session_id_from_hook_input(r#"{"session_id":"abc-1"}"#),
            "abc-1"
        );
        assert_eq!(session_id_from_hook_input("garbage"), "default");
        assert_eq!(
            session_id_from_hook_input(r#"{"session_id":"a b"}"#),
            "default"
        );
    }
}
