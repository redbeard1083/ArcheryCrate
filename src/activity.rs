//! Changes the tray process made while the window may be closed (charger plugged in or out,
//! tray menu), kept in a small file so the window's Activity page can show them.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ctl::Outcome;

const KEEP: usize = 60;

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("archerycrate").join("background-activity.jsonl")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Line {
    at: u64,
    outcome: Outcome,
}

pub fn append(o: &Outcome) {
    let p = path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let Ok(text) = serde_json::to_string(&Line { at: now(), outcome: o.clone() }) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{text}");
    }
    // Keep the file short.
    if let Ok(all) = std::fs::read_to_string(&p) {
        let lines: Vec<&str> = all.lines().collect();
        if lines.len() > KEEP * 2 {
            let _ = std::fs::write(&p, lines[lines.len() - KEEP..].join("\n") + "\n");
        }
    }
}

/// Newest first: (seconds ago, outcome).
pub fn recent() -> Vec<(u64, Outcome)> {
    let t = now();
    let mut v: Vec<(u64, Outcome)> = std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Line>(l).ok())
        .map(|l| (t.saturating_sub(l.at), l.outcome))
        .collect();
    v.reverse();
    v.truncate(KEEP);
    v
}
