//! The tray icon lives in its own small background process.
//!
//! On Wayland a program can't hide its window, so "close to tray" can't simply hide it.
//! Instead the tray icon runs in a separate process (`archerycrate --tray`) and the window
//! process really closes. The two talk over Unix sockets in the user's runtime folder:
//! the tray process asks the window to `show` or `quit`, and the window asks the tray
//! process to `quit`. A second window started while one is open just asks that one to show.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

use crate::ctl::{self, Msg, Tools};
use crate::hw::{Sampler, Snapshot};
use crate::tray::{self, TrayCmd};

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

pub fn tray_socket() -> PathBuf {
    runtime_dir().join("archerycrate-tray.sock")
}

pub fn window_socket() -> PathBuf {
    runtime_dir().join("archerycrate-window.sock")
}

/// Send one line to the process listening on `sock`; false if nobody is listening.
pub fn send(sock: &PathBuf, msg: &str) -> bool {
    match UnixStream::connect(sock) {
        Ok(mut s) => {
            let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
            s.write_all(format!("{msg}\n").as_bytes()).is_ok()
        }
        Err(_) => false,
    }
}

/// Listen on `sock` and hand every line to `on_line`. Fails with `AddrInUse` when another
/// live process already listens there.
pub fn listen(sock: &PathBuf, on_line: impl Fn(String) + Send + 'static) -> std::io::Result<()> {
    if UnixStream::connect(sock).is_ok() {
        return Err(std::io::ErrorKind::AddrInUse.into());
    }
    // Nobody answers: whatever is there is left over from a process that has ended.
    let _ = std::fs::remove_file(sock);
    let listener = UnixListener::bind(sock)?;
    std::thread::Builder::new().name("socket".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            for line in BufReader::new(conn).lines().map_while(Result::ok) {
                on_line(line.trim().to_string());
            }
        }
    })?;
    Ok(())
}

/// A command that starts this program again, detached from the current one.
/// From an AppImage it must go through the AppImage file: the current process's mount
/// disappears when it exits.
fn self_command(args: &[&str]) -> Option<Command> {
    let exe = ctl::own_appimage().or_else(|| std::env::current_exe().ok())?;
    let mut c = Command::new(exe);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    // Our own AppImage variables must not leak into the new process.
    c.env_remove("APPIMAGE").env_remove("APPDIR").env_remove("ARGV0").env_remove("OWD");
    // A token is single-use; only `open_window` passes a fresh one on.
    c.env_remove("XDG_ACTIVATION_TOKEN").env_remove("DESKTOP_STARTUP_ID");
    Some(c)
}

/// Start a program and collect it when it ends (in the background), so a closed window
/// doesn't stay behind as a "defunct" child of the tray process.
pub fn spawn_reaped(c: &mut Command) -> Result<(), String> {
    let mut child = c.spawn().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Start the tray process unless it's already running.
pub fn ensure_tray() -> Result<(), String> {
    if send(&tray_socket(), "ping") {
        return Ok(());
    }
    let mut c = self_command(&["--tray"]).ok_or("can't find the ArcheryCrate program")?;
    spawn_reaped(&mut c)
}

pub fn quit_tray() {
    send(&tray_socket(), "quit");
}

/// Raise the open window, or start one. A new window takes the desktop's activation token
/// from XDG_ACTIVATION_TOKEN, which is what lets it come up in front with focus on Wayland.
fn open_window(token: Option<String>) {
    if !send(&window_socket(), "show") {
        if let Some(mut c) = self_command(&[]) {
            if let Some(t) = token {
                c.env("XDG_ACTIVATION_TOKEN", t);
            }
            let _ = spawn_reaped(&mut c);
        }
    }
}

/// Ask the window to switch graphics (it shows the restart question), opening it if needed.
fn ask_window_gfx(m: ctl::GfxMode, token: Option<String>) {
    if !send(&window_socket(), &format!("gfx {}", m.key())) {
        if let Some(mut c) = self_command(&["--gfx", m.key()]) {
            if let Some(t) = token {
                c.env("XDG_ACTIVATION_TOKEN", t);
            }
            let _ = spawn_reaped(&mut c);
        }
    }
}

enum Event {
    Tray(TrayCmd),
    Tools(Tools),
    /// A fresh hardware reading from the sampler thread.
    Snap(Box<Snapshot>),
}

/// `archerycrate --tray`: the tray icon only, no window.
pub fn run_tray() {
    let (tx, rx) = channel::<Event>();
    let t2 = tx.clone();
    if listen(&tray_socket(), move |line| match line.as_str() {
        "quit" => {
            let _ = t2.send(Event::Tray(TrayCmd::Quit));
        }
        "show" => {
            let _ = t2.send(Event::Tray(TrayCmd::Show(None)));
        }
        _ => {}
    })
    .is_err()
    {
        return; // a tray process is already running
    }

    let (tray_tx, tray_rx) = channel::<TrayCmd>();
    let handle = match tray::start(tray_tx, || {}) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            let _ = std::fs::remove_file(tray_socket());
            return;
        }
    };
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for c in tray_rx {
                if tx.send(Event::Tray(c)).is_err() {
                    break;
                }
            }
        });
    }

    // asusctl availability, for changing the profile from the tray menu.
    let (msg_tx, msg_rx) = channel::<Msg>();
    ctl::detect_tools(msg_tx.clone(), || {});
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for m in msg_rx {
                match m {
                    Msg::Tools(t) => {
                        if tx.send(Event::Tools(t)).is_err() {
                            break;
                        }
                    }
                    // Shown on the window's Activity page.
                    Msg::Done(o) => crate::activity::append(&o),
                }
            }
        });
    }

    let mut tools = Tools::default();
    // Readings come from their own thread: a slow sensor or tool (e.g. while a monitor is being
    // plugged in and KDE reconfigures) must never stop the tray from answering clicks.
    {
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("tray-sampler".into())
            .spawn(move || {
                let mut sampler = Sampler::default();
                loop {
                    if tx.send(Event::Snap(Box::new(sampler.sample()))).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            })
            .ok();
    }
    let refresh = |s: Snapshot| {
        let profile = s.profile.clone().unwrap_or_default();
        let status = format!(
            "{} · CPU {} · Battery {}",
            crate::app::profile_title(&profile),
            s.cpu_temp_c.map(|t| format!("{t:.0}°C")).unwrap_or("–".into()),
            s.battery_pct.map(|b| format!("{b}%")).unwrap_or("–".into()),
        );
        let has_gfx = s.attr("gpu_mux_mode").is_some() || s.attr("dgpu_disable").is_some() || s.cardwire_mode.is_some();
        let gfx = has_gfx.then(|| s.gfx_mode());
        handle.update(|tr| {
            tr.profile = profile;
            tr.choices = s.profile_choices.clone();
            tr.status = status;
            tr.gfx = gfx;
        });
        s
    };
    // "60 Hz on battery" and "keyboard light off on battery" follow the charger even while
    // the window is closed. Wait until asusctl has been looked for, so a change never falls
    // back to a password prompt just because the check hadn't finished.
    let mut power = crate::display::PowerWatcher::default();
    let mut follow_charger = |on_ac: Option<bool>, tools: &Tools| {
        if !tools.checked {
            return;
        }
        if let Some(on_battery) = power.changed(on_ac) {
            crate::power::on_change(on_battery, tools, msg_tx.clone(), || {});
        }
    };
    let mut snap = Snapshot::default();
    loop {
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Event::Snap(s)) => {
                snap = refresh(*s);
                follow_charger(snap.on_ac, &tools);
            }
            Ok(Event::Tools(t)) => {
                tools = t;
                follow_charger(snap.on_ac, &tools);
            }
            Ok(Event::Tray(TrayCmd::Show(token))) => open_window(token),
            Ok(Event::Tray(TrayCmd::Gfx(m, token))) => {
                let now = snap.gfx_mode();
                let mux_now = snap.attr("gpu_mux_mode").and_then(|a| a.current);
                if m == now {
                    // already there
                } else if m == ctl::GfxMode::Ultimate || mux_now == Some(0) {
                    // Needs a restart: the window asks first ("restart now / later").
                    ask_window_gfx(m, token);
                } else {
                    ctl::execute(ctl::set_gfx(m, &snap.gfx_state()), tools.clone(), msg_tx.clone(), || {});
                    std::thread::sleep(Duration::from_millis(600));
                }
            }
            Ok(Event::Tray(TrayCmd::Profile(p))) => {
                ctl::execute(ctl::set_profile(&p), tools.clone(), msg_tx.clone(), || {});
                std::thread::sleep(Duration::from_millis(400));
            }
            Ok(Event::Tray(TrayCmd::Quit)) => {
                send(&window_socket(), "quit");
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // Leave cleanly: no socket for anyone to find, the tray icon taken off the panel (waited
    // for, but never longer than 2 s), then the process ends even if a helper thread lingers.
    let _ = std::fs::remove_file(tray_socket());
    let (done_tx, done_rx) = channel::<()>();
    std::thread::spawn(move || {
        handle.shutdown().wait();
        let _ = done_tx.send(());
    });
    let _ = done_rx.recv_timeout(Duration::from_secs(2));
    std::process::exit(0);
}
