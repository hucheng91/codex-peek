//! Single-instance + session lifecycle over a localhost TCP port.
//! Protocol: one line per connection: `start <id>`, `end <id>`, `ping`.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// Sends a command to a running instance. Returns false if none is listening.
pub fn send(port: u16, line: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr(port), Duration::from_millis(300)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
    if writeln!(s, "{line}").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Registers `id` on a connection that stays open; when this process exits
/// (even if killed), the OS closes the socket and the window drops the session.
pub fn attach(port: u16, id: &str) -> Option<TcpStream> {
    let mut s = TcpStream::connect_timeout(&addr(port), Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    writeln!(s, "attach {id}").ok()?;
    let mut reply = [0u8; 3];
    std::io::Read::read_exact(&mut s, &mut reply).ok()?;
    s.set_read_timeout(None).ok()?;
    (&reply == b"ok\n").then_some(s)
}

pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(addr(port))
}

#[derive(Debug, PartialEq)]
pub enum Msg {
    Start(String),
    End(String),
    Ping,
    /// Like Start, but the session ends when the connection closes.
    Attach(String),
}

pub fn parse_line(line: &str) -> Option<Msg> {
    let mut it = line.trim().splitn(2, ' ');
    match (it.next()?, it.next()) {
        ("start", Some(id)) => Some(Msg::Start(id.to_string())),
        ("end", Some(id)) => Some(Msg::End(id.to_string())),
        ("attach", Some(id)) => Some(Msg::Attach(id.to_string())),
        ("ping", None) => Some(Msg::Ping),
        _ => None,
    }
}

/// Serves connections forever, forwarding parsed messages to `on_msg`.
/// `attach <id>` is reported as `Start(id)` and, once the peer disconnects, `End(id)`.
pub fn serve(listener: TcpListener, on_msg: impl FnMut(Msg) + Send + 'static) {
    let on_msg = std::sync::Arc::new(std::sync::Mutex::new(on_msg));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let on_msg = std::sync::Arc::clone(&on_msg);
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let mut line = String::new();
                let mut r = BufReader::new(&stream);
                if r.read_line(&mut line).is_err() {
                    return;
                }
                let emit = |m: Msg| (on_msg.lock().unwrap())(m);
                match parse_line(&line) {
                    Some(Msg::Attach(id)) => {
                        emit(Msg::Start(id.clone()));
                        let _ = (&stream).write_all(b"ok\n");
                        let _ = stream.set_read_timeout(None);
                        let mut sink = Vec::new();
                        let _ = std::io::Read::read_to_end(&mut r, &mut sink);
                        emit(Msg::End(id));
                    }
                    Some(m) => {
                        emit(m);
                        let _ = (&stream).write_all(b"ok\n");
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
    use std::sync::mpsc;

    #[test]
    fn parses() {
        assert_eq!(parse_line("start abc\n"), Some(Msg::Start("abc".into())));
        assert_eq!(parse_line("end x"), Some(Msg::End("x".into())));
        assert_eq!(parse_line("ping"), Some(Msg::Ping));
        assert_eq!(parse_line("hello"), None);
    }

    #[test]
    fn roundtrip() {
        let l = bind(0).unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        serve(l, move |m| tx.send(m).unwrap());
        assert!(send(port, "start s1"));
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::Start("s1".into())
        );
        assert!(!send(port, "nonsense"));
        let conn = attach(port, "s2").expect("attach");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::Start("s2".into())
        );
        drop(conn);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Msg::End("s2".into())
        );
    }
}
