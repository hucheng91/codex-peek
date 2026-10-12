//! Single-instance + session lifecycle over a localhost TCP port.
//! Protocol: one line per connection:
//!   `start <id> [pid]`, `end <id>`, `ping`, `claimed`, `attach <id> [pid]`.
//! `attach` keeps the connection open; the session ends when it closes.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// Sends a command and returns the reply line (e.g. "ok", "ok 1 2").
pub fn query(port: u16, line: &str) -> Option<String> {
    let mut s = TcpStream::connect_timeout(&addr(port), Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    writeln!(s, "{line}").ok()?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).ok()?;
    let reply = reply.trim().to_string();
    reply.starts_with("ok").then_some(reply)
}

/// Sends a command. Returns false if no instance is listening.
pub fn send(port: u16, line: &str) -> bool {
    query(port, line).is_some()
}

/// Pids already claimed by sessions of the running window.
pub fn claimed(port: u16) -> Vec<u32> {
    query(port, "claimed")
        .map(|r| {
            r.split_whitespace()
                .skip(1)
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Registers `id` on a connection that stays open; when this process exits
/// (even if killed), the OS closes the socket and the window drops the session.
pub fn attach(port: u16, id: &str, pid: Option<u32>) -> Option<TcpStream> {
    let mut s = TcpStream::connect_timeout(&addr(port), Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    writeln!(
        s,
        "attach {id}{}",
        pid.map(|p| format!(" {p}")).unwrap_or_default()
    )
    .ok()?;
    let mut reply = [0u8; 3];
    std::io::Read::read_exact(&mut s, &mut reply).ok()?;
    s.set_read_timeout(None).ok()?;
    (&reply == b"ok\n").then_some(s)
}

pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr(port))
}

#[derive(Debug, PartialEq, Clone)]
pub enum Msg {
    Start(String, Option<u32>),
    End(String),
    Ping,
    Claimed,
    /// Like Start, but the session ends when the connection closes.
    Attach(String, Option<u32>),
}

pub fn parse_line(line: &str) -> Option<Msg> {
    let mut it = line.split_whitespace();
    let cmd = it.next()?;
    let id = it.next().map(str::to_string);
    let pid = match it.next() {
        Some(p) => Some(p.parse::<u32>().ok()?),
        None => None,
    };
    if it.next().is_some() {
        return None;
    }
    match (cmd, id) {
        ("start", Some(id)) => Some(Msg::Start(id, pid)),
        ("attach", Some(id)) => Some(Msg::Attach(id, pid)),
        ("end", Some(id)) if pid.is_none() => Some(Msg::End(id)),
        ("ping", None) => Some(Msg::Ping),
        ("claimed", None) => Some(Msg::Claimed),
        _ => None,
    }
}

/// Serves connections forever. `handler` returns the reply line for each
/// message; `attach` is delivered as `Start` and, on disconnect, `End`.
pub fn serve(listener: TcpListener, handler: impl Fn(Msg) -> String + Send + Sync + 'static) {
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = Arc::clone(&handler);
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                let mut r = BufReader::new(&stream);
                if r.read_line(&mut line).is_err() {
                    return;
                }
                match parse_line(&line) {
                    Some(Msg::Attach(id, pid)) => {
                        handler(Msg::Start(id.clone(), pid));
                        let _ = (&stream).write_all(b"ok\n");
                        let _ = stream.set_read_timeout(None);
                        let mut sink = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut r, &mut sink);
                        handler(Msg::End(id));
                    }
                    Some(m) => {
                        let reply = handler(m);
                        let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
                    }
                    None => {}
                }
            });
        }
    });
}

/// Launches `exe args` fully detached so it survives Codex killing the hook's
/// process tree (process group on Unix, Job Object on Windows; Codex creates
/// the job with JOB_OBJECT_LIMIT_BREAKAWAY_OK).
pub fn spawn_detached(exe: &std::path::Path, args: &[String]) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut c = Command::new(exe);
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe.
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        c.spawn().map(|_| ())
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        c.creation_flags(base | CREATE_BREAKAWAY_FROM_JOB);
        match c.spawn() {
            Ok(_) => Ok(()),
            Err(_) => {
                c.creation_flags(base);
                c.spawn().map(|_| ())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Mutex};

    #[test]
    fn parses() {
        assert_eq!(
            parse_line("start abc\n"),
            Some(Msg::Start("abc".into(), None))
        );
        assert_eq!(
            parse_line("start abc 42"),
            Some(Msg::Start("abc".into(), Some(42)))
        );
        assert_eq!(
            parse_line("attach x 7"),
            Some(Msg::Attach("x".into(), Some(7)))
        );
        assert_eq!(parse_line("end x"), Some(Msg::End("x".into())));
        assert_eq!(parse_line("ping"), Some(Msg::Ping));
        assert_eq!(parse_line("claimed"), Some(Msg::Claimed));
        assert_eq!(parse_line("start abc notapid"), None);
        assert_eq!(parse_line("hello"), None);
    }

    #[test]
    fn roundtrip() {
        let l = bind(0).unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        serve(l, move |m| {
            let reply = if m == Msg::Claimed {
                "ok 3 4".into()
            } else {
                "ok".into()
            };
            tx.lock().unwrap().send(m).unwrap();
            reply
        });
        assert!(send(port, "start s1 9"));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::Start("s1".into(), Some(9))
        );
        assert!(!send(port, "nonsense"));
        assert_eq!(claimed(port), vec![3, 4]);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let conn = attach(port, "s2", None).expect("attach");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::Start("s2".into(), None)
        );
        drop(conn);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::End("s2".into())
        );
    }
}
