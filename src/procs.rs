//! Finding the Codex TUI process that owns this session, so the window can
//! close when it exits.
//!
//! Embedded mode: the MCP server / hook is a descendant of the TUI.
//! Default mode (Codex >= 0.16x): a shared background `codex app-server`
//! daemon owns MCP servers and outlives the TUI, so we pick the youngest
//! terminal-attached `codex` process not already claimed by another session.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// Seconds since the process started.
    pub age: u64,
    pub has_tty: bool,
    /// Full command line.
    pub args: String,
}

impl Proc {
    fn exe_name(&self) -> String {
        let first = self.args.split_whitespace().next().unwrap_or("");
        let base = first.rsplit(['/', '\\']).next().unwrap_or(first);
        base.trim_matches('"').to_ascii_lowercase()
    }

    pub fn is_app_server(&self) -> bool {
        let name = self.exe_name();
        (name == "codex" || name == "codex.exe")
            && self.args.split_whitespace().nth(1) == Some("app-server")
    }

    /// An interactive Codex TUI (not the daemon, not `codex exec`/`mcp-server`/helpers).
    pub fn is_codex_tui(&self) -> bool {
        let name = self.exe_name();
        if name != "codex" && name != "codex.exe" {
            return false;
        }
        let sub = self.args.split_whitespace().nth(1).unwrap_or("");
        !matches!(
            sub,
            "app-server"
                | "exec"
                | "e"
                | "mcp-server"
                | "mcp"
                | "login"
                | "logout"
                | "plugin"
                | "completion"
                | "sandbox"
                | "debug"
                | "apply"
                | "cloud"
                | "proto"
                | "-V"
                | "--version"
                | "help"
        )
    }
}

/// Parses macOS/Linux `etime` ([[dd-]hh:]mm:ss) into seconds.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_etime(s: &str) -> Option<u64> {
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let parts: Vec<u64> = rest
        .split(':')
        .map(|p| p.parse().ok())
        .collect::<Option<_>>()?;
    let secs = match parts.as_slice() {
        [m, s] => m * 60 + s,
        [h, m, s] => h * 3600 + m * 60 + s,
        _ => return None,
    };
    Some(days * 86_400 + secs)
}

/// Parses `ps -axo pid=,ppid=,etime=,tty=,args=` output.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_ps(out: &str) -> Vec<Proc> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let age = parse_etime(it.next()?)?;
            let tty = it.next()?;
            let args = it.collect::<Vec<_>>().join(" ");
            Some(Proc {
                pid,
                ppid,
                age,
                has_tty: !matches!(tty, "?" | "??" | "-"),
                args,
            })
        })
        .collect()
}

/// Parses `pid|ppid|ageSeconds|commandline` lines (Windows PowerShell query).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_pipe_list(out: &str) -> Vec<Proc> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.trim().splitn(4, '|');
            Some(Proc {
                pid: it.next()?.parse().ok()?,
                ppid: it.next()?.parse().ok()?,
                age: it.next()?.parse().ok()?,
                has_tty: true, // no tty concept; TUI vs daemon is decided by args
                args: it.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Chooses the TUI for the process `me`.
pub fn choose_tui(procs: &[Proc], me: u32, claimed: &[u32]) -> Option<u32> {
    // 1. Nearest ancestor that is a Codex TUI (embedded mode).
    let mut cur = me;
    for _ in 0..64 {
        let Some(p) = procs.iter().find(|p| p.pid == cur) else {
            break;
        };
        if p.pid != me && p.is_codex_tui() {
            return Some(p.pid);
        }
        // A shared daemon's parent is whichever TUI happened to start it, not
        // necessarily ours, so stop walking here and use the fallback below.
        if p.is_app_server() {
            break;
        }
        if p.ppid == 0 || p.ppid == p.pid {
            break;
        }
        cur = p.ppid;
    }
    // 2. Youngest unclaimed terminal-attached TUI (shared daemon mode).
    procs
        .iter()
        .filter(|p| p.is_codex_tui() && p.has_tty && !claimed.contains(&p.pid))
        .min_by_key(|p| p.age)
        .map(|p| p.pid)
}

/// True if any Codex TUI is running (used for sessions with no known pid).
pub fn any_tui(procs: &[Proc]) -> bool {
    procs.iter().any(|p| p.is_codex_tui() && p.has_tty)
}

pub fn list() -> Vec<Proc> {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("ps")
            .args(["-axo", "pid=,ppid=,etime=,tty=,args="])
            .output();
        out.map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = "Get-CimInstance Win32_Process | ForEach-Object { $age = 0; if ($_.CreationDate) { $age = [int]((Get-Date) - $_.CreationDate).TotalSeconds }; \"$($_.ProcessId)|$($_.ParentProcessId)|$age|$($_.CommandLine)\" }";
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        out.map(|o| parse_pipe_list(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
}

/// Whether `pid` is still running.
pub fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks for existence/permission.
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        // SAFETY: plain Win32 calls on a handle we own and close.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return false;
            }
            let r = WaitForSingleObject(h, 0);
            CloseHandle(h);
            r == WAIT_TIMEOUT
        }
    }
}

/// How often the window re-checks watched processes.
pub const POLL: Duration = Duration::from_millis(1000);

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32, age: u64, tty: bool, args: &str) -> Proc {
        Proc {
            pid,
            ppid,
            age,
            has_tty: tty,
            args: args.into(),
        }
    }

    #[test]
    fn etime() {
        assert_eq!(parse_etime("05:07"), Some(307));
        assert_eq!(parse_etime("01:00:01"), Some(3601));
        assert_eq!(parse_etime("2-00:00:10"), Some(172_810));
        assert_eq!(parse_etime("x"), None);
    }

    #[test]
    fn ps_parsing() {
        let v = parse_ps("  10   1    00:05 pts/0    /usr/bin/codex --foo\n  11  1 1-02:03:04 ?  codex app-server --managed-daemon\nbad line\n");
        assert_eq!(v.len(), 2);
        assert!(v[0].has_tty && v[0].is_codex_tui());
        assert!(!v[1].has_tty && !v[1].is_codex_tui());
        assert_eq!(v[1].age, 93_784);
    }

    #[test]
    fn tui_classification() {
        assert!(p(1, 0, 0, true, r#""C:\x\codex.exe" --model y"#).is_codex_tui());
        assert!(p(1, 0, 0, true, "codex resume abc").is_codex_tui());
        assert!(!p(1, 0, 0, true, "node /x/bin/codex").is_codex_tui());
        assert!(!p(1, 0, 0, true, "codex exec hi").is_codex_tui());
        assert!(!p(1, 0, 0, true, "/x/codex-peek --mcp").is_codex_tui());
    }

    #[test]
    fn embedded_mode_uses_ancestor() {
        let procs = vec![
            p(100, 1, 50, true, "codex"),
            p(200, 1, 5, true, "codex"),
            p(101, 100, 40, false, "node bin/codex-peek-mcp.js"),
            p(102, 101, 40, false, "codex-peek --mcp"),
        ];
        assert_eq!(choose_tui(&procs, 102, &[]), Some(100));
    }

    #[test]
    fn daemon_mode_picks_youngest_unclaimed() {
        let procs = vec![
            p(9, 1, 900, false, "codex app-server --managed-daemon"),
            p(100, 50, 300, true, "codex"),
            p(200, 60, 4, true, "codex"),
            p(300, 9, 3, false, "node bin/codex-peek-mcp.js"),
            p(301, 300, 3, false, "codex-peek --mcp"),
        ];
        assert_eq!(choose_tui(&procs, 301, &[]), Some(200));
        assert_eq!(choose_tui(&procs, 301, &[200]), Some(100));
        assert_eq!(choose_tui(&procs, 301, &[200, 100]), None);
        // Daemon started by the older TUI 100: must not walk through it.
        let mut procs2 = procs.clone();
        procs2[0].ppid = 100;
        assert_eq!(choose_tui(&procs2, 301, &[]), Some(200));
        assert!(any_tui(&procs));
        assert!(!any_tui(&procs[..1]));
    }

    #[test]
    fn pipe_list() {
        let v = parse_pipe_list(
            "4|1|12|\"C:\\a\\codex.exe\"\r\n5|4|3|codex.exe app-server --managed-daemon\r\n",
        );
        assert_eq!(v.len(), 2);
        assert!(v[0].is_codex_tui() && !v[1].is_codex_tui());
    }

    #[test]
    fn self_is_alive() {
        assert!(alive(std::process::id()));
    }
}
