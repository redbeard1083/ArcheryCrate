//! The app was called ArmoryCraft before it became ArcheryCrate. On start, carry the
//! person's things over once: settings, the activity log, "start at login", and tidy away
//! the old launcher entry and icons (only ones the old app wrote itself). A still-running
//! old version is asked to quit, so there aren't two tray icons.

use std::path::{Path, PathBuf};

const OLD: &str = "armorycraft";
const OLD_ID: &str = "io.github.armorycraft.ArmoryCraft";

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".local/share")))
}

fn state_home() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".local/state")))
}

/// Rename `base/old` to `base/new` when only the old one exists.
fn move_dir(base: &Path, new: &str) {
    let (from, to) = (base.join(OLD), base.join(new));
    if from.is_dir() && !to.exists() {
        if let Err(e) = std::fs::rename(&from, &to) {
            eprintln!("could not move {} to {}: {e}", from.display(), to.display());
        }
    }
}

pub fn run() {
    // 1. Settings (fan curves, themes, options) and the battery state.
    if let Some(c) = config_home() {
        move_dir(&c, "archerycrate");
    }
    // 2. The tray's activity log.
    if let Some(s) = state_home() {
        move_dir(&s, "archerycrate");
    }
    // 3. "Start when I log in": rewrite it for the new name and program.
    if let Some(c) = config_home() {
        let old = c.join("autostart").join(format!("{OLD}.desktop"));
        if old.exists() {
            if !crate::tray::autostart_enabled() {
                let _ = crate::tray::set_autostart(true);
            }
            let _ = std::fs::remove_file(old);
        }
    }
    // 4. The launcher entry and icons the old AppImage registered (it marked its entry).
    if let Some(d) = data_home() {
        let entry = d.join("applications").join(format!("{OLD_ID}.desktop"));
        let ours = std::fs::read_to_string(&entry).is_ok_and(|t| t.contains("X-ArmoryCraft-AppImage=true"));
        if ours {
            let _ = std::fs::remove_file(&entry);
            let icons = d.join("icons/hicolor");
            for size in ["scalable", "32x32", "48x48", "64x64", "128x128", "256x256"] {
                for ext in ["svg", "png"] {
                    let _ = std::fs::remove_file(icons.join(size).join("apps").join(format!("{OLD_ID}.{ext}")));
                }
            }
        }
    }
    // 5. An old ArmoryCraft still running: ask its tray and window to quit.
    let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    for sock in [format!("{OLD}-tray.sock"), format!("{OLD}-window.sock")] {
        crate::daemon::send(&run.join(sock), "quit");
    }
}
