#![cfg_attr(windows, windows_subsystem = "windows")]
//! codex-peek: floating always-on-top preview of images pasted into Codex CLI.

mod cli;
mod control;
mod matcher;
mod mcp;
mod procs;
mod sessions;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::http::{header::CONTENT_TYPE, Response};
use wry::WebViewBuilder;

use cli::{Mode, Options};
use control::Msg;
use matcher::ImageList;
use sessions::Sessions;

const SMALL: (f64, f64) = (430.0, 140.0);
const LARGE: (f64, f64) = (720.0, 540.0);

#[derive(Debug)]
enum UserEvent {
    Tick,
    Ipc(String),
    Show,
    Quit,
}

fn main() {
    let opts = match cli::parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{}", cli::HELP);
            std::process::exit(2);
        }
    };
    match opts.mode {
        Mode::Help => print!("{}", cli::HELP),
        Mode::Version => println!("codex-peek {}", env!("CARGO_PKG_VERSION")),
        Mode::HookStart => hook_start(&opts),
        Mode::HookStop => {
            let id = read_session_id();
            control::send(opts.port, &format!("end {id}"));
        }
        Mode::Mcp => run_mcp(&opts),
        Mode::Run => run(opts),
    }
}

fn read_session_id() -> String {
    let mut s = String::new();
    let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s);
    cli::session_id_from_hook_input(&s)
}

/// SessionStart hook: register with a running window, or spawn one detached.
/// Always exits 0 quickly so it never blocks Codex.
fn hook_start(opts: &Options) {
    let id = read_session_id();
    // Codex runs SessionStart lazily (on the first turn), so images pasted into
    // the first prompt already exist: include recent files when we launch.
    let pid = find_tui(opts);
    let msg = format!(
        "start {id}{}",
        pid.map(|p| format!(" {p}")).unwrap_or_default()
    );
    ensure_window(opts, &msg, &["--existing"]);
}

/// Sends `msg` to the running window, launching one (detached) if needed.
fn ensure_window(opts: &Options, msg: &str, extra: &[&str]) {
    if control::send(opts.port, msg) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut args = vec![
        "--managed".to_string(),
        "--port".into(),
        opts.port.to_string(),
    ];
    for a in std::env::args().skip(1) {
        if !matches!(a.as_str(), "--hook-start" | "--mcp" | "--existing") {
            args.push(a);
        }
    }
    args.extend(extra.iter().map(|s| s.to_string()));
    if control::spawn_detached(&exe, &args).is_err() {
        return;
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(150));
        if control::send(opts.port, msg) {
            return;
        }
    }
}

/// Minimal stdio MCP server. Codex starts plugin MCP servers when the session
/// boots and closes stdin when it exits, which gives an exact lifecycle for the
/// preview window (earlier than the SessionStart hook, which waits for a turn).
fn run_mcp(opts: &Options) {
    use std::io::{BufRead, Write};
    let id = format!("mcp-{}", std::process::id());
    ensure_window(opts, "ping", &[]);
    // Two exit signals: (1) the Codex TUI pid, watched by the window, which
    // matters when a shared `codex app-server` daemon owns this MCP server and
    // outlives the TUI; (2) this socket, which closes if Codex kills us.
    let pid = find_tui(opts);
    let _link = control::attach(opts.port, &id, pid);
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Some(reply) = mcp::respond(&line) {
            let _ = writeln!(out, "{reply}");
            let _ = out.flush();
        }
    }
}

/// Identifies the Codex TUI for this session (see `procs`), skipping TUIs
/// already claimed by other sessions of the running window.
fn find_tui(opts: &Options) -> Option<u32> {
    let claimed = control::claimed(opts.port);
    procs::choose_tui(&procs::list(), std::process::id(), &claimed)
}

fn state_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("codex-peek").join("position"))
}

fn load_position() -> Option<PhysicalPosition<i32>> {
    let s = std::fs::read_to_string(state_file()?).ok()?;
    let (x, y) = s.trim().split_once(',')?;
    Some(PhysicalPosition::new(x.parse().ok()?, y.parse().ok()?))
}

fn save_position(p: PhysicalPosition<i32>) {
    if let Some(f) = state_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, format!("{},{}", p.x, p.y));
    }
}

fn run(opts: Options) {
    // Single instance: if one is already running, just register and leave.
    let listener = match control::bind(opts.port) {
        Ok(l) => l,
        Err(_) => {
            eprintln!("codex-peek is already running (port {}).", opts.port);
            return;
        }
    };
    let dir = opts.dir.clone().unwrap_or_else(matcher::default_dir);
    let pattern = opts
        .pattern
        .clone()
        .unwrap_or_else(|| matcher::DEFAULT_PATTERN.into());
    let since = if opts.include_existing {
        SystemTime::now() - Duration::from_secs(600)
    } else {
        SystemTime::now() - Duration::from_secs(1)
    };
    eprintln!("codex-peek: watching {} for {}", dir.display(), pattern);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let mut wb = WindowBuilder::new()
        .with_title("Codex Peek")
        .with_decorations(false)
        .with_always_on_top(true)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(SMALL.0, SMALL.1));
    if let Some(p) = load_position() {
        wb = wb.with_position(p);
    }
    let window = wb.build(&event_loop).expect("create window");

    let shown: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let shown_proto = Arc::clone(&shown);
    let p_ipc = proxy.clone();

    let builder = WebViewBuilder::new()
        .with_html(include_str!("../ui/index.html"))
        .with_transparent(true)
        .with_ipc_handler(move |req| {
            let _ = p_ipc.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .with_custom_protocol("peek".into(), move |_id, req| {
            let idx: Option<usize> = req.uri().path().trim_start_matches("/img/").parse().ok();
            let path = idx.and_then(|i| shown_proto.lock().unwrap().get(i).cloned());
            match path.and_then(|p| std::fs::read(p).ok()) {
                Some(bytes) => Response::builder()
                    .header(CONTENT_TYPE, "image/png")
                    .body(bytes.into())
                    .unwrap(),
                None => Response::builder()
                    .status(404)
                    .body(Vec::new().into())
                    .unwrap(),
            }
        });

    #[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd")))]
    let webview = builder.build(&window).expect("create webview");
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd"))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder
            .build_gtk(window.default_vbox().unwrap())
            .expect("create webview")
    };

    let registry: Arc<Mutex<Sessions>> = Arc::default();
    let reg = Arc::clone(&registry);
    let p_ctl = proxy.clone();
    control::serve(listener, move |m| {
        let mut s = reg.lock().unwrap();
        match m {
            Msg::Start(id, pid) => {
                s.start(&id, pid);
                let _ = p_ctl.send_event(UserEvent::Show);
            }
            Msg::End(id) => s.end(&id),
            Msg::Claimed => {
                let pids: Vec<String> = s.claimed().iter().map(u32::to_string).collect();
                return format!("ok {}", pids.join(" ")).trim_end().to_string();
            }
            Msg::Ping | Msg::Attach(..) => {}
        }
        "ok".into()
    });
    // Lifecycle thread: reap sessions whose Codex TUI exited, fall back to
    // "no Codex TUI running at all" for sessions without a known pid, and
    // quit (when launched by Codex) once the last session is gone.
    if opts.managed {
        let reg = Arc::clone(&registry);
        let p_life = proxy.clone();
        std::thread::spawn(move || {
            let mut n = 0u32;
            loop {
                std::thread::sleep(procs::POLL);
                n += 1;
                let pidless = {
                    let mut s = reg.lock().unwrap();
                    s.reap_dead(procs::alive);
                    s.has_pidless()
                };
                if pidless && n.is_multiple_of(5) {
                    let any = procs::any_tui(&procs::list());
                    reg.lock().unwrap().check_orphans(any);
                }
                if reg.lock().unwrap().all_ended() {
                    let _ = p_life.send_event(UserEvent::Quit);
                    break;
                }
            }
        });
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(400));
        if proxy.send_event(UserEvent::Tick).is_err() {
            break;
        }
    });

    let url_base = if cfg!(windows) {
        "http://peek.localhost"
    } else {
        "peek://localhost"
    };
    let mut list = ImageList::new();
    let mut generation = 0u64;
    let mut pending_pos: Option<PhysicalPosition<i32>> = None;
    let mut ready = false;
    let mut restore_pos: Option<PhysicalPosition<i32>> = None;
    let mut suppress_save = false;

    let render = move |list: &ImageList, generation: u64, webview: &wry::WebView| {
        *shown.lock().unwrap() = list.entries().iter().map(|e| e.path.clone()).collect();
        let items: Vec<serde_json::Value> = list
            .entries()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                serde_json::json!({
                    "n": e.number,
                    "name": e.path.file_name().map(|n| n.to_string_lossy().into_owned()),
                    "url": format!("{url_base}/img/{i}?g={generation}"),
                })
            })
            .collect();
        let _ = webview.evaluate_script(&format!("render({})", serde_json::Value::from(items)));
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Tick) => {
                let now = Instant::now();
                let mut changed = false;
                if list.should_auto_reset(now, opts.idle_reset) {
                    list.clear();
                    generation += 1;
                    changed = true;
                }
                for p in matcher::scan(&dir, &pattern, since) {
                    changed |= list.add(p, now).is_some();
                }
                if changed && ready {
                    render(&list, generation, &webview);
                }
                if let Some(p) = pending_pos.take() {
                    save_position(p);
                }
            }
            Event::UserEvent(UserEvent::Ipc(m)) => match m.as_str() {
                "ready" => {
                    ready = true;
                    render(&list, generation, &webview);
                }
                "drag" => {
                    let _ = window.drag_window();
                }
                "clear" => {
                    list.clear();
                    generation += 1;
                    render(&list, generation, &webview);
                }
                "enlarge" => {
                    // Grow in place, but shift so the large view stays on screen.
                    let pos = window.outer_position().ok();
                    restore_pos = pos;
                    if let (Some(p), Some(mon)) = (pos, window.current_monitor()) {
                        let scale = window.scale_factor();
                        let (mp, ms) = (mon.position(), mon.size());
                        let w = (LARGE.0 * scale) as i32;
                        let h = (LARGE.1 * scale) as i32;
                        let x = p.x.min(mp.x + ms.width as i32 - w).max(mp.x);
                        let y = p.y.min(mp.y + ms.height as i32 - h).max(mp.y);
                        window.set_inner_size(LogicalSize::new(LARGE.0, LARGE.1));
                        if (x, y) != (p.x, p.y) {
                            suppress_save = true;
                            window.set_outer_position(PhysicalPosition::new(x, y));
                        }
                    } else {
                        window.set_inner_size(LogicalSize::new(LARGE.0, LARGE.1));
                    }
                }
                "shrink" => {
                    window.set_inner_size(LogicalSize::new(SMALL.0, SMALL.1));
                    if let Some(p) = restore_pos.take() {
                        window.set_outer_position(p);
                    }
                    suppress_save = false;
                }
                "close" => *control_flow = ControlFlow::Exit,
                _ => {}
            },
            Event::UserEvent(UserEvent::Show) => window.set_visible(true),
            Event::UserEvent(UserEvent::Quit) => *control_flow = ControlFlow::Exit,
            Event::WindowEvent {
                event: WindowEvent::Moved(p),
                ..
            } => {
                if !suppress_save {
                    pending_pos = Some(p)
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            _ => {}
        }
    });
}
