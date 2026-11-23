//! "60 Hz on battery": the laptop screen's refresh rate follows the charger.
//!
//! Uses KDE Plasma's `kscreen-doctor` (Wayland and X11). Only the refresh rate changes;
//! the resolution in use is kept.

use crate::ctl::run_host;

#[derive(Clone, Debug)]
struct Mode {
    id: String,
    w: u64,
    h: u64,
    hz: f64,
}

#[derive(Clone, Debug)]
pub struct Panel {
    name: String,
    current: Mode,
    modes: Vec<Mode>,
}

impl Panel {
    pub fn describe(&self) -> String {
        format!("{}×{} at {:.0} Hz", self.current.w, self.current.h, self.current.hz)
    }

    /// The fastest rate at the current resolution (what the charger brings back).
    pub fn max_hz(&self) -> f64 {
        self.same_size().map(|m| m.hz).fold(0.0, f64::max)
    }

    fn same_size(&self) -> impl Iterator<Item = &Mode> {
        self.modes.iter().filter(|m| m.w == self.current.w && m.h == self.current.h)
    }

    fn target(&self, on_battery: bool) -> Option<&Mode> {
        let key = |m: &&Mode| if on_battery { -(m.hz - 60.0).abs() } else { m.hz };
        self.same_size().max_by(|a, b| key(a).total_cmp(&key(b)))
    }
}

/// The built-in screen, read from kscreen-doctor. None when it isn't available.
pub fn panel() -> Option<Panel> {
    let json = run_host(&["kscreen-doctor", "-j"]).ok()?;
    let v: serde_json::Value = serde_json::from_str(json.get(json.find('{')?..)?).ok()?;
    let mode = |m: &serde_json::Value| -> Option<Mode> {
        Some(Mode {
            id: match &m["id"] {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            },
            w: m["size"]["width"].as_u64()?,
            h: m["size"]["height"].as_u64()?,
            hz: m["refreshRate"].as_f64()?,
        })
    };
    v["outputs"].as_array()?.iter().find_map(|o| {
        let name = o["name"].as_str()?.to_string();
        // Type 7 is KScreen's "Panel"; the name check covers older versions.
        let internal = o["type"].as_u64() == Some(7) || name.starts_with("eDP") || name.starts_with("LVDS");
        if !internal || o["connected"].as_bool() == Some(false) || o["enabled"].as_bool() == Some(false) {
            return None;
        }
        let modes: Vec<Mode> = o["modes"].as_array()?.iter().filter_map(mode).collect();
        let cur_id = match &o["currentModeId"] {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let current = modes.iter().find(|m| m.id == cur_id)?.clone();
        Some(Panel { name, current, modes })
    })
}

/// Set the screen for battery (≈60 Hz) or charger (fastest) use. Returns what was done.
pub fn apply(on_battery: bool) -> Result<String, String> {
    let p = panel().ok_or("The screen can't be changed here (needs KDE Plasma's kscreen-doctor).")?;
    let t = p.target(on_battery).ok_or("No matching display mode.")?;
    if t.id == p.current.id {
        return Ok(format!("Screen already at {:.0} Hz", t.hz));
    }
    run_host(&["kscreen-doctor", &format!("output.{}.mode.{}", p.name, t.id)])?;
    Ok(format!("Screen → {:.0} Hz ({})", t.hz, if on_battery { "on battery" } else { "on the charger" }))
}

// ---------------------------------------------------------------------------
// Screen brightness
// ---------------------------------------------------------------------------

const KDE_BUS: &str = "org.kde.ScreenBrightness";
const KDE_DISPLAY: &str = "org.kde.ScreenBrightness.Display";

/// `busctl` prints a property as e.g. `i 10000` or `as 1 "display1"`.
fn busctl_get(path: &str, iface: &str, prop: &str) -> Option<String> {
    run_host(&["busctl", "--user", "get-property", KDE_BUS, path, iface, prop]).ok()
}

/// KDE's object for the built-in screen (keeps KDE's own brightness slider in step).
fn kde_internal_display() -> Option<String> {
    let names = busctl_get("/org/kde/ScreenBrightness", KDE_BUS, "DisplaysDBusNames")?;
    names.split('"').skip(1).step_by(2).map(|n| format!("/org/kde/ScreenBrightness/{n}")).find(|p| {
        busctl_get(p, KDE_DISPLAY, "IsInternal").is_some_and(|v| v.trim() == "b true")
    })
}

fn int_prop(path: &str, prop: &str) -> Option<i64> {
    busctl_get(path, KDE_DISPLAY, prop)?.split_whitespace().nth(1)?.parse().ok()
}

/// The laptop screen's backlight device in /sys/class/backlight.
fn sys_backlight() -> Option<std::path::PathBuf> {
    std::fs::read_dir("/sys/class/backlight").ok()?.flatten().map(|e| e.path()).next()
}

/// Screen brightness in percent.
pub fn backlight_pct() -> Option<u8> {
    if let Some(p) = kde_internal_display() {
        let (b, m) = (int_prop(&p, "Brightness")?, int_prop(&p, "MaxBrightness")?);
        if m > 0 {
            return Some(((b * 100 + m / 2) / m).clamp(0, 100) as u8);
        }
    }
    let d = sys_backlight()?;
    let b: i64 = crate::hw::read_num(d.join("brightness"))?;
    let m: i64 = crate::hw::read_num(d.join("max_brightness"))?;
    (m > 0).then(|| ((b * 100 + m / 2) / m).clamp(0, 100) as u8)
}

/// Set the screen brightness in percent: through KDE when it runs, otherwise through
/// logind (both work without a password).
pub fn set_backlight_pct(pct: u8) -> Result<String, String> {
    let pct = pct.clamp(1, 100) as i64;
    if let Some(p) = kde_internal_display() {
        let m = int_prop(&p, "MaxBrightness").ok_or("KDE didn't report the screen's range")?;
        let v = (m * pct / 100).max(1).to_string();
        run_host(&["busctl", "--user", "call", KDE_BUS, &p, KDE_DISPLAY, "SetBrightness", "iu", &v, "0"])?;
        return Ok(format!("Screen brightness → {pct}%"));
    }
    let d = sys_backlight().ok_or("No screen backlight found")?;
    let m: i64 = crate::hw::read_num(d.join("max_brightness")).ok_or("Unknown backlight range")?;
    let name = d.file_name().and_then(|n| n.to_str()).ok_or("Bad backlight name")?.to_string();
    let v = (m * pct / 100).max(1).to_string();
    run_host(&[
        "busctl", "call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session", "SetBrightness", "ssu", "backlight", &name, &v,
    ])?;
    Ok(format!("Screen brightness → {pct}%"))
}

/// Acts when the charger is plugged in or out (and once at the start).
#[derive(Default)]
pub struct PowerWatcher {
    last: Option<bool>,
}

impl PowerWatcher {
    /// Returns Some(on_battery) when the screen should be switched now.
    pub fn changed(&mut self, on_ac: Option<bool>) -> Option<bool> {
        let ac = on_ac?;
        if self.last == Some(ac) {
            return None;
        }
        self.last = Some(ac);
        Some(!ac)
    }
}
