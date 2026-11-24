//! What happens when the charger is plugged in or out (the Battery page's "On battery" card):
//! performance mode, fan curves, graphics, keyboard light, screen brightness and refresh rate.
//!
//! Runs in the tray process (so it works with the window closed), or in the window when the
//! tray is turned off. Every change arrives as an ordinary `Msg::Done` outcome.
//!
//! What was there before unplugging is kept in `battery-state.json`, a small file of its own,
//! so the window and the tray process (each with their own settings copy) don't overwrite it.
//! Only what was changed is put back, even if its option was turned off in between.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

use crate::ctl::{self, GfxMode, Msg, Outcome, Tools};
use crate::hw::{Curve, Sampler};
use crate::settings::{self, Settings};

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    fans: Option<SavedFans>,
    /// The graphics mode was Standard before Eco.
    #[serde(default)]
    gfx_standard: bool,
    #[serde(default)]
    kbd: Option<u8>,
    #[serde(default)]
    backlight: Option<u8>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedFans {
    profile: String,
    cpu: Curve,
    gpu: Curve,
    enabled: bool,
}

fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("archerycrate").join("battery-state.json")
}

impl Saved {
    fn load() -> Self {
        let mut s: Saved = std::fs::read_to_string(state_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // From the first version: the keyboard level kept in its own file.
        let old = state_path().with_file_name("keyboard-before-battery");
        if let Some(l) = std::fs::read_to_string(&old).ok().and_then(|t| t.trim().parse().ok()) {
            s.kbd.get_or_insert(l);
            let _ = std::fs::remove_file(old);
        }
        s
    }

    fn save(&self) {
        let p = state_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, t);
        }
    }
}

/// Which battery options to act on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Profile,
    Fans,
    Gfx,
    Keyboard,
    Backlight,
    Refresh,
}

/// The charger was plugged in (`on_battery == false`) or out. Runs in the background.
pub fn on_change(on_battery: bool, tools: &Tools, tx: Sender<Msg>, repaint: impl Fn() + Send + 'static) {
    let tools = tools.clone();
    std::thread::spawn(move || {
        let send = |o: Outcome| {
            let _ = tx.send(Msg::Done(o));
            repaint();
        };
        if on_battery {
            go_battery(&tools, None, &send);
        } else {
            go_charger(&tools, None, &send);
        }
    });
}

/// One option was turned on or off while on battery: apply or undo just that one.
pub fn toggle(item: Item, on: bool, tools: &Tools, tx: Sender<Msg>, repaint: impl Fn() + Send + 'static) {
    let tools = tools.clone();
    std::thread::spawn(move || {
        let send = |o: Outcome| {
            let _ = tx.send(Msg::Done(o));
            repaint();
        };
        // Levels (keyboard, brightness) may have changed: undo first, then apply the new one.
        if !on || matches!(item, Item::Keyboard | Item::Backlight) {
            go_charger(&tools, Some(item), &send);
        }
        if on {
            go_battery(&tools, Some(item), &send);
        }
    });
}

fn note(label: &str, ok: bool, text: String) -> Outcome {
    Outcome { label: label.into(), ok, via: String::new(), log: vec![text], needs_reboot: false }
}

fn go_battery(tools: &Tools, only: Option<Item>, send: &dyn Fn(Outcome)) {
    let s = Settings::load();
    let mut saved = Saved::load();
    let snap = Sampler::default().sample();
    let want = |i: Item, setting: bool| only.map_or(setting, |o| o == i);
    let current = snap.profile.clone().unwrap_or_default();

    // 1. Performance mode → Silent.
    let mut profile_now = current.clone();
    if want(Item::Profile, s.battery_silent) && !current.is_empty() && current != "quiet" {
        let mut a = ctl::set_profile("quiet");
        a.label = "Performance → Silent (on battery)".into();
        let o = ctl::run_action(a, tools);
        if o.ok {
            saved.profile.get_or_insert(current.clone());
            profile_now = "quiet".into();
        }
        send(o);
    }

    // 2. Silent fan curves in the mode in use (Silent mode has them already).
    if want(Item::Fans, s.battery_silent_fans) && profile_now != "quiet" && !profile_now.is_empty() && saved.fans.is_none() {
        if let Some((cpu, gpu, enabled)) = ctl::read_fan_state(&profile_now) {
            // Silent's curve as the Fans page shows it: yours when you made one, else the
            // laptop's stock Silent curve, else a gentle fallback shape.
            let mut cfg = s.clone();
            let quiet = cfg.curves_for("quiet").clone();
            let (qc, qg) = if quiet.enabled {
                (quiet.cpu, quiet.gpu)
            } else if let Some(st) = s.stock.get("quiet") {
                (st.cpu, st.gpu)
            } else {
                (settings::default_curve("quiet", false), settings::default_curve("quiet", true))
            };
            let mut a = ctl::apply_fan_curves(&profile_now, &qc, &qg, true, snap.curve_hwmon.as_deref());
            a.label = "Fans → silent curves (on battery)".into();
            let o = ctl::run_action(a, tools);
            if o.ok {
                saved.fans = Some(SavedFans { profile: profile_now.clone(), cpu, gpu, enabled });
            }
            send(o);
        } else {
            send(note("Fans → silent curves (on battery)", false, "asusctl didn't report the fan curves".into()));
        }
    }

    // 3. Graphics → Eco.
    if want(Item::Gfx, s.battery_eco) {
        match snap.gfx_mode() {
            GfxMode::Standard => {
                let mut a = ctl::set_gfx(GfxMode::Eco, &snap.gfx_state());
                a.label = "Graphics → Eco (on battery)".into();
                let o = ctl::run_action(a, tools);
                if o.ok {
                    saved.gfx_standard = true;
                }
                send(o);
            }
            GfxMode::Ultimate => send(note(
                "Graphics stays Ultimate",
                false,
                "Eco on battery can't switch from Ultimate without a restart.".into(),
            )),
            GfxMode::Eco => {}
        }
    }

    // 4. Keyboard light: off, or dimmed to the chosen level.
    let kbd_target = if !want(Item::Keyboard, s.kbd_off_on_battery || s.battery_kbd_dim) {
        None
    } else if s.kbd_off_on_battery {
        Some(0)
    } else if s.battery_kbd_dim {
        Some(s.battery_kbd_level.clamp(1, 3))
    } else {
        None
    };
    if let Some(target) = kbd_target {
        let now = snap.kbd_brightness.unwrap_or(0);
        if now > target {
            let mut a = ctl::set_kbd_brightness(target);
            a.label = if target == 0 {
                "Keyboard light off (on battery)".into()
            } else {
                format!("Keyboard light → {} (on battery)", ctl::kbd_level_name(target))
            };
            let o = ctl::run_action(a, tools);
            if o.ok {
                saved.kbd.get_or_insert(now);
            }
            send(o);
        }
    }

    // 5. Screen brightness.
    if want(Item::Backlight, s.battery_backlight) {
        let target = s.battery_backlight_pct.clamp(5, 100);
        if let Some(now) = crate::display::backlight_pct() {
            if now > target {
                match crate::display::set_backlight_pct(target) {
                    Ok(t) => {
                        saved.backlight.get_or_insert(now);
                        send(note("Screen brightness (on battery)", true, t));
                    }
                    Err(e) => send(note("Screen brightness (on battery)", false, e)),
                }
            }
        }
    }

    // 6. Refresh rate → 60 Hz.
    if want(Item::Refresh, s.battery_60hz) {
        match crate::display::apply(true) {
            Ok(t) => {
                saved.refresh = true;
                send(note("Screen refresh → 60 Hz", true, t));
            }
            Err(e) => send(note("Screen refresh → 60 Hz", false, e)),
        }
    }
    saved.save();
}

/// Put back what `go_battery` changed (only `only`, when given).
///
/// Graphics come back first: on the G14 the HDMI port is wired to the discrete GPU, so an
/// external monitor plugged in with the charger needs it — and the screen steps below go
/// through KDE's display tools, which can be slow while KDE sets that monitor up. The state
/// file is saved after every step, so nothing is put back twice or forgotten.
fn go_charger(tools: &Tools, only: Option<Item>, send: &dyn Fn(Outcome)) {
    let mut saved = Saved::load();
    let want = |i: Item| only.is_none_or(|o| o == i);

    if want(Item::Gfx) && std::mem::take(&mut saved.gfx_standard) {
        saved.save();
        let snap = Sampler::default().sample();
        if snap.gfx_mode() == GfxMode::Eco {
            let mut a = ctl::set_gfx(GfxMode::Standard, &snap.gfx_state());
            a.label = "Graphics back to Standard".into();
            send(ctl::run_action(a, tools));
        }
    }
    if want(Item::Profile) {
        if let Some(p) = saved.profile.take() {
            saved.save();
            let mut a = ctl::set_profile(&p);
            a.label = format!("Performance back to {}", crate::app::profile_title(&p));
            send(ctl::run_action(a, tools));
        }
    }
    if want(Item::Keyboard) {
        if let Some(level) = saved.kbd.take() {
            saved.save();
            let mut a = ctl::set_kbd_brightness(level);
            a.label = format!("Keyboard light back to {}", ctl::kbd_level_name(level));
            send(ctl::run_action(a, tools));
        }
    }
    if want(Item::Backlight) {
        if let Some(pct) = saved.backlight.take() {
            saved.save();
            match crate::display::set_backlight_pct(pct) {
                Ok(t) => send(note("Screen brightness back", true, t)),
                Err(e) => send(note("Screen brightness back", false, e)),
            }
        }
    }
    if want(Item::Refresh) && std::mem::take(&mut saved.refresh) {
        saved.save();
        match crate::display::apply(false) {
            Ok(t) => send(note("Screen refresh → full speed", true, t)),
            Err(e) => send(note("Screen refresh → full speed", false, e)),
        }
    }
    if want(Item::Fans) {
        if let Some(f) = saved.fans.take() {
            saved.save();
            let snap = Sampler::default().sample();
            let active = snap.profile.as_deref() == Some(f.profile.as_str());
            let hw = snap.curve_hwmon;
            let mut a = ctl::apply_fan_curves(&f.profile, &f.cpu, &f.gpu, active, hw.as_deref());
            a.label = format!("Fan curves back for {}", crate::app::profile_title(&f.profile));
            if !f.enabled {
                // The curve data goes back too; then the laptop's own curves take over again.
                let off = ctl::fan_curves_off(&f.profile, active, hw.as_deref());
                a.groups.extend(off.groups);
            }
            send(ctl::run_action(a, tools));
        }
    }
    saved.save();
}
