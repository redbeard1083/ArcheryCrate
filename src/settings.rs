//! Remembered choices (fan curves per profile, lighting) in ~/.config/archerycrate/settings.json.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ctl::Aura;
use crate::hw::Curve;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileCurves {
    pub cpu: Curve,
    pub gpu: Curve,
    pub enabled: bool,
}

/// The laptop's factory fan curves for a mode, read once (after `asusctl fan-curve --default`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StockCurves {
    pub cpu: Curve,
    pub gpu: Curve,
}

/// The window's colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    /// Light or dark, following the desktop's setting.
    #[default]
    System,
    Dark,
    Light,
    /// Cyberpunk: dark with neon colours, glow and a moving rainbow.
    Neon,
    /// The same neon look on a light background.
    NeonLight,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: Theme,
    /// Neon without the moving rainbow.
    #[serde(default)]
    pub neon_calm: bool,
    /// How fast Neon's rainbow moves, as a multiple of the normal speed (None = normal).
    #[serde(default)]
    pub rainbow_speed: Option<f32>,
    /// Interface size as a multiple (0.8…1.1); None = fit the window (shrinks on small screens).
    #[serde(default)]
    pub ui_scale: Option<f32>,
    /// How much history Home's graphs show, in seconds (60, 300 or 900; None = 60).
    #[serde(default)]
    pub home_range_secs: Option<u32>,
    /// Home as a 3×3 grid that fits the window (instead of the detailed, scrolling page).
    #[serde(default)]
    pub home_grid: bool,
    /// Neon without the moving rainbow while on battery (Battery › On battery).
    #[serde(default)]
    pub battery_calm: bool,
    /// Your own curves per mode; `enabled` = the fans use them (otherwise the stock curves).
    #[serde(default)]
    pub curves: BTreeMap<String, ProfileCurves>,
    /// Stock curves per mode, as read from the laptop.
    #[serde(default)]
    pub stock: BTreeMap<String, StockCurves>,
    #[serde(default)]
    pub aura: Aura,
    #[serde(default)]
    pub anime_on: bool,
    #[serde(default)]
    pub anime_brightness: u8,
    /// Closing the window keeps ArcheryCrate running in the tray.
    #[serde(default = "yes")]
    pub close_to_tray: bool,
    /// Show the tray icon at all.
    #[serde(default = "yes")]
    pub tray_icon: bool,
    /// Drop the laptop screen to 60 Hz on battery, back to its fastest rate on the charger.
    #[serde(default)]
    pub battery_60hz: bool,
    /// Turn the keyboard light off on battery and bring it back on the charger.
    #[serde(default)]
    pub kbd_off_on_battery: bool,
    /// Dim the keyboard light on battery to `battery_kbd_level` (1 low … 3 high).
    #[serde(default)]
    pub battery_kbd_dim: bool,
    #[serde(default = "one")]
    pub battery_kbd_level: u8,
    /// Set the screen brightness to `battery_backlight_pct` on battery.
    #[serde(default)]
    pub battery_backlight: bool,
    #[serde(default = "fifty")]
    pub battery_backlight_pct: u8,
    /// Graphics to Eco on battery (back to Standard on the charger).
    #[serde(default)]
    pub battery_eco: bool,
    /// Performance mode Silent on battery (back to the previous mode on the charger).
    #[serde(default)]
    pub battery_silent: bool,
    /// Silent fan curves on battery, whatever the performance mode.
    #[serde(default)]
    pub battery_silent_fans: bool,
}

fn one() -> u8 {
    1
}

fn fifty() -> u8 {
    50
}

fn yes() -> bool {
    true
}

/// The fixed temperature columns every fan curve uses in the editor (°C).
pub const FAN_COLUMNS: [u8; 8] = [30, 40, 50, 60, 70, 80, 90, 100];

/// A curve put onto `FAN_COLUMNS`: the fan speed the original curve gives at each column's
/// temperature (the first point's speed below it, the last point's above it, a straight line
/// between points). Curves from the laptop can repeat points (Silent's stock curve has
/// 60,65,65,70,70,75,75,98), so this is what makes all modes edit the same way.
pub fn on_columns(c: &Curve) -> Curve {
    let mut pts: Vec<(f32, f32)> = c.iter().map(|(t, d)| (*t as f32, *d as f32)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let speed_at = |t: f32| -> f32 {
        if t <= pts[0].0 {
            return pts[0].1;
        }
        for w in pts.windows(2) {
            let ((t0, d0), (t1, d1)) = (w[0], w[1]);
            if t <= t1 {
                return if t1 > t0 { d0 + (d1 - d0) * (t - t0) / (t1 - t0) } else { d1 };
            }
        }
        pts[pts.len() - 1].1
    };
    let mut out: Curve = [(0, 0); 8];
    let mut floor = 0u8;
    for (i, t) in FAN_COLUMNS.iter().enumerate() {
        let d = (speed_at(*t as f32).round().clamp(0.0, 100.0) as u8).max(floor);
        floor = d;
        out[i] = (*t, d);
    }
    out
}

/// Whether a curve already sits on `FAN_COLUMNS`.
pub fn is_on_columns(c: &Curve) -> bool {
    c.iter().zip(FAN_COLUMNS).all(|((t, _), col)| *t == col)
}

pub fn default_curve(profile: &str, gpu: bool) -> Curve {
    let base: Curve = match profile {
        "quiet" | "low-power" => [(30, 0), (40, 0), (50, 5), (60, 15), (70, 30), (80, 45), (90, 60), (100, 70)],
        "performance" => [(30, 15), (40, 20), (50, 30), (60, 45), (70, 60), (80, 75), (90, 90), (100, 100)],
        _ => [(30, 5), (40, 10), (50, 20), (60, 30), (70, 45), (80, 60), (90, 75), (100, 85)],
    };
    if gpu {
        let mut g = base;
        for p in g.iter_mut() {
            p.1 = (p.1 + 5).min(100);
        }
        g
    } else {
        base
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("archerycrate").join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = std::fs::read_to_string(path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::from_str("{}").unwrap_or_default());
        // Keyboard off and dimmed on battery exclude each other (older versions allowed both;
        // "off" won then, so keep that).
        if s.kbd_off_on_battery && s.battery_kbd_dim {
            s.battery_kbd_dim = false;
        }
        s
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, t);
        }
    }

    pub fn curves_for(&mut self, profile: &str) -> &mut ProfileCurves {
        self.curves.entry(profile.to_string()).or_insert_with(|| ProfileCurves {
            cpu: default_curve(profile, false),
            gpu: default_curve(profile, true),
            enabled: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepped_stock_curve_goes_onto_columns() {
        // The G14's stock Silent CPU curve: repeated points make steps.
        let pct = |p: u32| ((p * 100 + 127) / 255) as u8;
        let silent: Curve = [
            (60, pct(7)),
            (65, pct(30)),
            (65, pct(30)),
            (70, pct(56)),
            (70, pct(56)),
            (75, pct(86)),
            (75, pct(86)),
            (98, pct(104)),
        ];
        let c = on_columns(&silent);
        assert!(is_on_columns(&c));
        // Below the first point it holds the first point's speed; above the last, the last.
        assert_eq!(c[0], (30, pct(7)));
        assert_eq!(c[7], (100, pct(104)));
        // Never falls.
        assert!(c.windows(2).all(|w| w[0].1 <= w[1].1));
        // A curve already on the columns is unchanged.
        assert_eq!(on_columns(&c), c);
    }
}
