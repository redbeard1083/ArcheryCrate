//! Making changes: asusctl first, a privileged sysfs write as fallback.
//!
//! Every change is an [`Action`]: a list of command groups (each group is a list of
//! alternative command lines for different asusctl versions; the first that succeeds
//! wins, and every group must succeed), plus the sysfs writes that do the same thing
//! directly. Actions run on a background thread and report an [`Outcome`].

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::hw::Curve;

#[derive(Clone, Debug, Default)]
pub struct Action {
    pub label: String,
    pub groups: Vec<Vec<Vec<String>>>,
    pub sysfs: Vec<(PathBuf, String)>,
    pub needs_reboot: bool,
    /// Commands run, in order, after the change above succeeded (all must succeed).
    pub post: Vec<Vec<String>>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Outcome {
    pub label: String,
    pub ok: bool,
    pub via: String,
    pub log: Vec<String>,
    pub needs_reboot: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Tools {
    pub checked: bool,
    pub flatpak: bool,
    pub appimage: bool,
    pub asusctl: Option<String>,
    pub pkexec: bool,
    /// asusd (the service asusctl talks to) is running.
    pub asusd_active: bool,
    /// Cardwire: None = not installed, Some(true) = its service answers.
    pub cardwire: Option<bool>,
    /// How Cardwire could be installed (looked up only when it's missing).
    pub cardwire_install: Option<crate::setup::CardwireInstall>,
}

pub enum Msg {
    Tools(Tools),
    Done(Outcome),
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// The path of our own AppImage, only when this process really runs from it
/// (APPIMAGE can leak into child processes of other AppImages).
pub fn own_appimage() -> Option<std::path::PathBuf> {
    let appdir = std::env::var_os("APPDIR")?;
    let exe = std::env::current_exe().ok()?;
    if exe.starts_with(&appdir) { std::env::var_os("APPIMAGE").map(Into::into) } else { None }
}

pub fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Inside the Flatpak sandbox the host tools are reached through flatpak-spawn.
fn host_command(args: &[String]) -> Command {
    if in_flatpak() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host").args(args);
        c
    } else {
        let mut c = Command::new(&args[0]);
        c.args(&args[1..]);
        c
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let out = host_command(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not start {}: {e}", args[0]))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .trim()
    .to_string();
    // Some asusctl versions print usage and still exit 0 on a bad subcommand.
    let looks_wrong = text.contains("Unrecognised")
        || text.contains("Unrecognized")
        || text.contains("unrecognized")
        // asusctl 6.5 can print a D-Bus error and still exit 0 (e.g. `leds set` on this G14).
        || text.contains("DBus.Error")
        || text.lines().any(|l| l.trim_start().starts_with("Error:"))
        || text.contains("unexpected argument")
        || text.contains("Usage:") && text.contains("error");
    if out.status.success() && !looks_wrong {
        Ok(text)
    } else {
        Err(if text.is_empty() { format!("exit status {}", out.status) } else { text })
    }
}

/// Pull a version number such as "6.5.0" out of a tool's output.
fn version_in(text: &str) -> Option<String> {
    // Prefer an explicit "version: x.y.z" line, then any word that looks like a version.
    let lines: Vec<&str> = text.lines().collect();
    let pick = lines
        .iter()
        .find(|l| l.to_ascii_lowercase().contains("version"))
        .or_else(|| lines.iter().find(|l| l.chars().any(|c| c.is_ascii_digit())))?;
    pick.split(|c: char| c.is_whitespace() || c == ':')
        .map(|w| w.trim_start_matches(['v', 'V']))
        .find(|w| w.contains('.') && w.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|w| w.to_string())
}

/// Whether a program is on the (host's) PATH, even if it has no version flag.
fn on_path(cmd: &str) -> bool {
    if in_flatpak() {
        return run(&argv(&["sh", "-c", &format!("command -v {cmd}")])).is_ok();
    }
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&dirs)
        .chain(["/usr/bin", "/usr/local/bin", "/bin"].iter().map(PathBuf::from))
        .any(|d| d.join(cmd).is_file())
}

/// Run a host program and return its output (through flatpak-spawn inside Flatpak).
pub fn run_host(args: &[&str]) -> Result<String, String> {
    run(&argv(args))
}

/// Some tools know `--version`; asusctl 6.x only answers `asusctl info`.
fn version_of(cmd: &str) -> Option<String> {
    for flag in ["--version", "info", "-V"] {
        if let Ok(t) = run(&argv(&[cmd, flag])) {
            return Some(version_in(&t).unwrap_or_else(|| "installed".into()));
        }
    }
    on_path(cmd).then(|| "installed".into())
}

pub fn detect_tools(tx: Sender<Msg>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let mut t = Tools {
            checked: true,
            flatpak: in_flatpak(),
            appimage: own_appimage().is_some(),
            ..Default::default()
        };
        t.asusctl = version_of("asusctl");
        t.pkexec = run(&argv(&["pkexec", "--version"])).is_ok();
        let active = |svc: &str| {
            run(&argv(&["systemctl", "is-active", svc])).map(|o| o.trim() == "active").unwrap_or(false)
        };
        t.asusd_active = active("asusd");
        if on_path("cardwire") {
            t.cardwire = Some(run(&argv(&["cardwire", "manager", "status"])).is_ok());
        }
        // Also when it's installed: the setup card shows the full install command.
        t.cardwire_install = Some(crate::setup::cardwire_install(crate::setup::detect().family));
        let _ = tx.send(Msg::Tools(t));
        repaint();
    });
}

fn safe_value(v: &str) -> bool {
    !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || " _.-".contains(c))
}

fn sysfs_write(writes: &[(PathBuf, String)], log: &mut Vec<String>) -> Result<(), String> {
    let mut script = String::new();
    for (p, v) in writes {
        let ps = p.to_string_lossy();
        if !ps.starts_with("/sys/") || ps.contains('\'') || !safe_value(v) {
            return Err(format!("refused unsafe write {ps} = {v}"));
        }
        if !script.is_empty() {
            script.push_str(" && ");
        }
        script.push_str(&format!("printf '%s' '{v}' > '{ps}'"));
        log.push(format!("write {v} → {ps}"));
    }
    // Try without elevation first (works when the user has been given access), then pkexec.
    let direct = writes.iter().all(|(p, v)| std::fs::write(p, v).is_ok());
    if direct {
        return Ok(());
    }
    run(&argv(&["pkexec", "sh", "-c", &script])).map(|_| ())
}

pub fn execute(action: Action, tools: Tools, tx: Sender<Msg>, repaint: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        let _ = tx.send(Msg::Done(run_action(action, &tools)));
        repaint();
    });
}

/// Run one action to the end on the current thread (for steps that must happen in order).
pub fn run_action(action: Action, tools: &Tools) -> Outcome {
    {
        let start = Instant::now();
        let mut log = Vec::new();
        let mut ok = false;
        let mut via = String::new();

        let uses_tool = |g: &Vec<Vec<String>>| {
            g.first().map(|c| c[0].clone()).unwrap_or_default()
        };
        let tool_available = |name: &str| match name {
            "asusctl" => tools.asusctl.is_some(),
            _ => true,
        };

        if !action.groups.is_empty() && action.groups.iter().all(|g| tool_available(&uses_tool(g))) {
            let mut all = true;
            for group in &action.groups {
                let mut group_ok = false;
                for alt in group {
                    log.push(format!("$ {}", alt.join(" ")));
                    match run(alt) {
                        Ok(out) => {
                            if !out.is_empty() {
                                log.push(out.lines().take(6).collect::<Vec<_>>().join("\n"));
                            }
                            group_ok = true;
                            break;
                        }
                        Err(e) => log.push(format!("  ✗ {}", e.lines().take(3).collect::<Vec<_>>().join(" / "))),
                    }
                }
                if !group_ok {
                    all = false;
                    break;
                }
            }
            if all {
                ok = true;
                via = uses_tool(&action.groups[0]);
            }
        } else if !action.groups.is_empty() {
            log.push(format!("{} is not installed", uses_tool(&action.groups[0])));
        }

        if !ok && !action.sysfs.is_empty() {
            log.push("falling back to a direct hardware write (asks for your password)".into());
            match sysfs_write(&action.sysfs, &mut log) {
                Ok(()) => {
                    ok = true;
                    via = "sysfs".into();
                }
                Err(e) => log.push(format!("  ✗ {e}")),
            }
        }
        if !ok && action.groups.is_empty() && action.sysfs.is_empty() && !action.post.is_empty() {
            ok = true; // nothing to do before the follow-up steps
        }
        if ok {
            for cmd in &action.post {
                log.push(format!("$ {}", cmd.join(" ")));
                match run(cmd) {
                    Ok(out) => {
                        if !out.is_empty() {
                            log.push(out.lines().take(4).collect::<Vec<_>>().join("\n"));
                        }
                        if via.is_empty() {
                            via = cmd[0].clone();
                        }
                    }
                    Err(e) => {
                        log.push(format!("  ✗ {}", e.lines().take(3).collect::<Vec<_>>().join(" / ")));
                        ok = false;
                        break;
                    }
                }
            }
        }
        log.push(format!("took {} ms", start.elapsed().as_millis()));
        Outcome { label: action.label, ok, via, log, needs_reboot: action.needs_reboot }
    }
}

// ---------------------------------------------------------------------------
// The actions the UI offers
// ---------------------------------------------------------------------------

/// platform_profile name → asusctl name
pub fn asusctl_profile(p: &str) -> &'static str {
    match p {
        "quiet" | "low-power" => "Quiet",
        "performance" => "Performance",
        _ => "Balanced",
    }
}

pub fn set_profile(profile: &str) -> Action {
    let name = asusctl_profile(profile);
    Action {
        label: format!("Performance mode → {}", crate::app::profile_title(profile)),
        groups: vec![vec![
            argv(&["asusctl", "profile", "set", name]),
            argv(&["asusctl", "profile", "--profile-set", name]),
            argv(&["asusctl", "profile", "-P", name]),
        ]],
        sysfs: vec![(crate::hw::PLATFORM_PROFILE.into(), profile.to_string())],
        ..Default::default()
    }
}

pub fn set_charge_limit(limit: u8, path: Option<PathBuf>) -> Action {
    let n = limit.to_string();
    Action {
        label: format!("Battery charge limit → {limit}%"),
        groups: vec![vec![
            argv(&["asusctl", "battery", "limit", &n]),
            argv(&["asusctl", "--chg-limit", &n]),
            argv(&["asusctl", "-c", &n]),
        ]],
        sysfs: path.map(|p| vec![(p, n.clone())]).unwrap_or_default(),
        ..Default::default()
    }
}

pub fn one_shot_charge() -> Action {
    Action {
        label: "Charge to 100% once".into(),
        groups: vec![vec![
            argv(&["asusctl", "battery", "oneshot"]),
            argv(&["asusctl", "--one-shot-chg"]),
            argv(&["asusctl", "-o"]),
        ]],
        ..Default::default()
    }
}

pub fn kbd_level_name(level: u8) -> &'static str {
    match level {
        0 => "off",
        1 => "low",
        2 => "med",
        _ => "high",
    }
}

pub fn set_kbd_brightness(level: u8) -> Action {
    let name = kbd_level_name(level);
    let led = std::path::Path::new(crate::hw::KBD_LED)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("asus::kbd_backlight")
        .to_string();
    let lv = level.to_string();
    Action {
        label: format!("Keyboard brightness → {name}"),
        // logind first: it sets the LED for the signed-in person without a password.
        // (asusctl 6.5's `leds set` fails here with "Attribute not found: brightness".)
        groups: vec![vec![
            argv(&[
                "busctl", "call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session", "SetBrightness", "ssu", "leds", &led, &lv,
            ]),
            argv(&["asusctl", "leds", "set", name]),
            argv(&["asusctl", "--kbd-bright", name]),
            argv(&["asusctl", "-k", name]),
        ]],
        sysfs: vec![(
            PathBuf::from(crate::hw::KBD_LED).join("brightness"),
            level.to_string(),
        )],
        ..Default::default()
    }
}

fn curve_data(c: &Curve) -> String {
    c.iter().map(|(t, d)| format!("{t}c:{d}%")).collect::<Vec<_>>().join(",")
}

pub fn apply_fan_curves(profile: &str, cpu: &Curve, gpu: &Curve, current_profile: bool, hwmon: Option<&Path>) -> Action {
    let p = asusctl_profile(profile);
    let mut groups = Vec::new();
    for (fan, c) in [("cpu", cpu), ("gpu", gpu)] {
        let d = curve_data(c);
        groups.push(vec![
            argv(&["asusctl", "fan-curve", "--mod-profile", p, "--fan", fan, "--data", &d]),
            argv(&["asusctl", "fan-curve", "-m", p, "-f", fan, "-D", &d]),
        ]);
    }
    groups.push(vec![
        argv(&["asusctl", "fan-curve", "--mod-profile", p, "--enable-fan-curves", "true"]),
        argv(&["asusctl", "fan-curve", "-m", p, "-e", "true"]),
    ]);
    let mut sysfs = Vec::new();
    if let (true, Some(h)) = (current_profile, hwmon) {
        for (pwm, c) in [(1, cpu), (2, gpu)] {
            for (i, (t, d)) in c.iter().enumerate() {
                sysfs.push((h.join(format!("pwm{pwm}_auto_point{}_temp", i + 1)), t.to_string()));
                let raw = (*d as u32 * 255 / 100).min(255);
                sysfs.push((h.join(format!("pwm{pwm}_auto_point{}_pwm", i + 1)), raw.to_string()));
            }
            sysfs.push((h.join(format!("pwm{pwm}_enable")), "1".into()));
        }
    }
    Action {
        label: format!("Custom fan curves on for {}", crate::app::profile_title(profile)),
        groups,
        sysfs,
        ..Default::default()
    }
}

/// Stop using custom curves for `profile` (the laptop's own curves take over); the stored
/// curve data is kept.
pub fn fan_curves_off(profile: &str, current_profile: bool, hwmon: Option<&Path>) -> Action {
    let p = asusctl_profile(profile);
    let mut sysfs = Vec::new();
    if let (true, Some(h)) = (current_profile, hwmon) {
        sysfs.push((h.join("pwm1_enable"), "2".into()));
        sysfs.push((h.join("pwm2_enable"), "2".into()));
    }
    Action {
        label: format!("Laptop's own fan curves for {}", crate::app::profile_title(profile)),
        groups: vec![vec![
            argv(&["asusctl", "fan-curve", "--mod-profile", p, "--enable-fan-curves", "false"]),
            argv(&["asusctl", "fan-curve", "-m", p, "-e", "false"]),
        ]],
        sysfs,
        ..Default::default()
    }
}

/// Back to the laptop's own (factory) fan curves for `profile`. `active` is the mode in use.
pub fn reset_fan_curves(profile: &str, active: &str, hwmon: Option<&Path>) -> Action {
    let p = asusctl_profile(profile);
    let current_profile = profile == active;
    let mut sysfs = Vec::new();
    if let (true, Some(h)) = (current_profile, hwmon) {
        sysfs.push((h.join("pwm1_enable"), "2".into()));
        sysfs.push((h.join("pwm2_enable"), "2".into()));
    }
    let mut groups = Vec::new();
    // asusctl 6.5: `--default` resets only the *active* mode's curve data to the factory one,
    // so for another mode: switch to it for a moment, reset, and switch back.
    let switch_away = !current_profile && !active.is_empty();
    if switch_away {
        groups.push(vec![argv(&["asusctl", "profile", "set", p])]);
    }
    if current_profile || switch_away {
        groups.push(vec![
            argv(&["asusctl", "fan-curve", "--default"]),
            argv(&["asusctl", "fan-curve", "-m", p, "-d"]),
        ]);
    }
    groups.push(vec![
        argv(&["asusctl", "fan-curve", "--mod-profile", p, "--enable-fan-curves", "false"]),
        argv(&["asusctl", "fan-curve", "-m", p, "-e", "false"]),
    ]);
    if switch_away {
        groups.push(vec![argv(&["asusctl", "profile", "set", asusctl_profile(active)])]);
    }
    Action {
        label: format!("{FAN_RESET_LABEL} {}", crate::app::profile_title(profile)),
        groups,
        sysfs,
        ..Default::default()
    }
}

/// Every fan-reset action's label starts with this.
pub const FAN_RESET_LABEL: &str = "Fan curves back to default for";

/// The curves asusctl holds for a mode (`asusctl fan-curve --mod-profile <mode>`):
/// CPU first, then GPU, as (°C, %) points.
pub fn read_fan_curves(profile: &str) -> Option<(Curve, Curve)> {
    read_fan_state(profile).map(|(c, g, _)| (c, g))
}

/// The curves asusctl holds for a mode, and whether they're switched on.
pub fn read_fan_state(profile: &str) -> Option<(Curve, Curve, bool)> {
    let out = run(&argv(&["asusctl", "fan-curve", "--mod-profile", asusctl_profile(profile)])).ok()?;
    let (c, g) = parse_fan_curves(&out)?;
    Some((c, g, out.contains("enabled: true")))
}

fn parse_fan_curves(out: &str) -> Option<(Curve, Curve)> {
    let tuple_after = |text: &str, key: &str| -> Option<(Vec<u32>, usize)> {
        let at = text.find(key)? + key.len();
        let end = at + text[at..].find(')')?;
        let v: Vec<u32> = text[at..end].split(',').filter_map(|x| x.trim().parse().ok()).collect();
        Some((v, end))
    };
    let mut curves = Vec::new();
    let mut rest = out;
    for _ in 0..2 {
        let (pwm, e1) = tuple_after(rest, "pwm: (")?;
        let (temp, e2) = tuple_after(rest, "temp: (")?;
        if pwm.len() != 8 || temp.len() != 8 {
            return None;
        }
        let mut c: Curve = [(0, 0); 8];
        for i in 0..8 {
            c[i] = (temp[i].min(255) as u8, ((pwm[i] * 100 + 127) / 255).min(100) as u8);
        }
        curves.push(c);
        rest = &rest[e1.max(e2)..];
    }
    Some((curves[0], curves[1]))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum AuraMode {
    Static,
    Breathe,
    Pulse,
    RainbowCycle,
    RainbowWave,
}

impl AuraMode {
    pub const ALL: [AuraMode; 5] = [
        AuraMode::Static,
        AuraMode::Breathe,
        AuraMode::Pulse,
        AuraMode::RainbowCycle,
        AuraMode::RainbowWave,
    ];
    pub fn title(self) -> &'static str {
        match self {
            AuraMode::Static => "Static",
            AuraMode::Breathe => "Breathing",
            AuraMode::Pulse => "Pulse",
            AuraMode::RainbowCycle => "Color cycle",
            AuraMode::RainbowWave => "Rainbow wave",
        }
    }
    fn cli(self) -> &'static str {
        match self {
            AuraMode::Static => "static",
            AuraMode::Breathe => "breathe",
            AuraMode::Pulse => "pulse",
            AuraMode::RainbowCycle => "rainbow-cycle",
            AuraMode::RainbowWave => "rainbow-wave",
        }
    }
    pub fn uses_color(self) -> bool {
        matches!(self, AuraMode::Static | AuraMode::Breathe | AuraMode::Pulse)
    }
    pub fn uses_speed(self) -> bool {
        matches!(self, AuraMode::Breathe | AuraMode::RainbowCycle | AuraMode::RainbowWave)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Aura {
    pub mode: AuraMode,
    pub color1: [u8; 3],
    pub color2: [u8; 3],
    pub speed: u8, // 0 low, 1 med, 2 high
    pub direction: String,
    pub awake: bool,
    pub boot: bool,
    pub sleep: bool,
    pub shutdown: bool,
}

impl Default for Aura {
    fn default() -> Self {
        Self {
            mode: AuraMode::Static,
            color1: [255, 32, 64],
            color2: [32, 96, 255],
            speed: 1,
            direction: "right".into(),
            awake: true,
            boot: true,
            sleep: false,
            shutdown: false,
        }
    }
}

impl Aura {
    /// Same effect, colours, speed and direction (the power-state switches aside).
    pub fn same_effect(&self, o: &Aura) -> bool {
        self.mode == o.mode
            && self.color1 == o.color1
            && self.color2 == o.color2
            && self.speed == o.speed
            && self.direction == o.direction
    }
}

/// Every lighting-effect action's label starts with this.
pub const AURA_LABEL: &str = "Keyboard lighting →";

fn hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn set_aura(a: &Aura) -> Action {
    let speed = ["low", "med", "high"][a.speed.min(2) as usize].to_string();
    // asusctl 6.x spelling (checked against `asusctl aura effect <mode> --help`, v6.5.0).
    let mut long: Vec<String> = Vec::new();
    // Older asusctl versions used short flags.
    let mut short: Vec<String> = Vec::new();
    if a.mode.uses_color() {
        long.extend(["--colour".into(), hex(a.color1)]);
        short.extend(["-c".into(), hex(a.color1)]);
    }
    if a.mode == AuraMode::Breathe {
        long.extend(["--colour2".into(), hex(a.color2)]);
        short.extend(["-C".into(), hex(a.color2)]);
    }
    if a.mode.uses_speed() {
        long.extend(["--speed".into(), speed.clone()]);
        short.extend(["-s".into(), speed]);
    }
    if a.mode == AuraMode::RainbowWave {
        long.extend(["--direction".into(), a.direction.clone()]);
        short.extend(["-d".into(), a.direction.clone()]);
    }
    // Static and pulse only take -c in 6.x.
    if matches!(a.mode, AuraMode::Static | AuraMode::Pulse) {
        long = short.clone();
    }
    let mk = |prefix: &[&str], args: &[String]| {
        let mut v = argv(prefix);
        v.push(a.mode.cli().into());
        v.extend(args.iter().cloned());
        v
    };
    Action {
        label: format!("{AURA_LABEL} {}", a.mode.title()),
        groups: vec![vec![
            mk(&["asusctl", "aura", "effect"], &long),
            mk(&["asusctl", "aura"], &short),
            mk(&["asusctl", "led-mode"], &short),
        ]],
        ..Default::default()
    }
}

pub fn set_aura_power(a: &Aura) -> Action {
    let b = |v: bool| if v { "true" } else { "false" };
    // asusctl 6.x: on/off switches, a missing switch means "off".
    let mut v6 = argv(&["asusctl", "aura", "power", "keyboard"]);
    for (on, flag) in [(a.boot, "--boot"), (a.awake, "--awake"), (a.sleep, "--sleep"), (a.shutdown, "--shutdown")] {
        if on {
            v6.push(flag.into());
        }
    }
    Action {
        label: "Keyboard lighting power states".into(),
        groups: vec![vec![
            v6,
            argv(&[
                "asusctl", "aura-power", "keyboard", "--awake", b(a.awake), "--boot", b(a.boot), "--sleep",
                b(a.sleep), "--shutdown", b(a.shutdown),
            ]),
            argv(&[
                "asusctl", "leds-power", "keyboard", "--awake", b(a.awake), "--boot", b(a.boot), "--sleep",
                b(a.sleep), "--shutdown", b(a.shutdown),
            ]),
            argv(&[
                "asusctl", "led-pow-2", "keyboard", "-a", b(a.awake), "-b", b(a.boot), "-s", b(a.sleep), "-d",
                b(a.shutdown),
            ]),
        ]],
        ..Default::default()
    }
}

pub fn anime_enable(on: bool) -> Action {
    let b = if on { "true" } else { "false" };
    Action {
        label: format!("Lid matrix display {}", if on { "on" } else { "off" }),
        groups: vec![vec![
            argv(&["asusctl", "anime", "--enable-display", b]),
            argv(&["asusctl", "anime", "-e", b]),
        ]],
        ..Default::default()
    }
}

pub fn anime_brightness(level: u8) -> Action {
    let name = kbd_level_name(level);
    Action {
        label: format!("Lid matrix brightness → {name}"),
        groups: vec![vec![
            argv(&["asusctl", "anime", "--brightness", name]),
            argv(&["asusctl", "anime", "-b", name]),
        ]],
        ..Default::default()
    }
}

/// A BIOS / firmware attribute (panel overdrive, boot sound, power limits...).
pub fn set_attr(name: &str, path: &Path, value: i64) -> Action {
    let v = value.to_string();
    let b = if value != 0 { "true" } else { "false" };
    let mut alts = vec![
        argv(&["asusctl", "armoury", "set", name, &v]),
        argv(&["asusctl", "armoury", name, &v]),
    ];
    match name {
        "panel_overdrive" => alts.push(argv(&["asusctl", "bios", "--panel-overdrive-set", b])),
        "boot_sound" => alts.push(argv(&["asusctl", "bios", "--post-sound-set", b])),
        _ => {}
    }
    Action {
        label: format!("{} → {}", crate::app::attr_title(name), v),
        groups: vec![alts],
        sysfs: vec![(path.to_path_buf(), v.clone())],
        needs_reboot: matches!(name, "gpu_mux_mode"),
        ..Default::default()
    }
}

/// Every graphics-mode action's label starts with this.
pub const GFX_LABEL: &str = "Graphics mode →";

/// Restart the laptop (asks the system's session manager; no password for the logged-in person).
pub fn restart_laptop() -> Action {
    Action {
        label: "Restarting the laptop".into(),
        groups: vec![vec![argv(&["systemctl", "reboot"]), argv(&["loginctl", "reboot"])]],
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GfxMode {
    Eco,
    Standard,
    Ultimate,
}

impl GfxMode {
    pub const ALL: [GfxMode; 3] = [GfxMode::Eco, GfxMode::Standard, GfxMode::Ultimate];

    /// The word used between the tray and the window ("eco", "standard", "ultimate").
    pub fn key(self) -> &'static str {
        match self {
            GfxMode::Eco => "eco",
            GfxMode::Standard => "standard",
            GfxMode::Ultimate => "ultimate",
        }
    }

    pub fn from_key(k: &str) -> Option<GfxMode> {
        GfxMode::ALL.into_iter().find(|m| m.key() == k.trim().to_lowercase())
    }

    pub fn title(self) -> &'static str {
        match self {
            GfxMode::Eco => "Eco",
            GfxMode::Standard => "Standard",
            GfxMode::Ultimate => "Ultimate",
        }
    }
}

/// What the graphics switch needs to know about the laptop right now.
#[derive(Clone, Debug, Default)]
pub struct GfxState {
    pub dgpu_disable: Option<PathBuf>,
    /// 1 when the firmware has cut the discrete GPU off the bus.
    pub dgpu_disabled: Option<i64>,
    pub mux: Option<PathBuf>,
    /// 0 = Ultimate (screen wired to the discrete GPU).
    pub mux_now: Option<i64>,
    /// Cardwire's daemon runs: Eco/Standard block and unblock the discrete GPU in software.
    pub cardwire: bool,
}

fn cardwire(mode: &str) -> Vec<String> {
    argv(&["cardwire", "set", mode])
}

pub fn set_gfx(mode: GfxMode, st: &GfxState) -> Action {
    let mux_now = st.mux_now;
    let dgpu_off = st.dgpu_disabled == Some(1);
    let mut sysfs: Vec<(PathBuf, String)> = Vec::new();
    let mut post = Vec::new();
    // Bringing back a GPU the firmware cut off needs the switch cleared *and* a PCI rescan
    // (without it the GPU stays missing until a restart). Needs root: one password prompt.
    let revive = |sysfs: &mut Vec<(PathBuf, String)>| {
        if let (true, Some(d)) = (dgpu_off, &st.dgpu_disable) {
            sysfs.push((d.clone(), "0".into()));
            sysfs.push((PathBuf::from("/sys/bus/pci/rescan"), "1".into()));
        }
    };
    match mode {
        GfxMode::Eco => {
            if mux_now == Some(0) {
                // In Ultimate the discrete GPU drives the screen: switching it off now would
                // pull the display away. Hand the screen back first; Eco can follow after
                // the restart (the UI explains this).
                if let Some(m) = &st.mux {
                    sysfs.push((m.clone(), "1".into()));
                }
            } else if st.cardwire {
                // Cardwire blocks the discrete GPU and lets it power down; it stays on the
                // bus, so switching back is instant (the firmware switch couldn't come back).
                revive(&mut sysfs);
                post.push(cardwire("integrated"));
            } else if let Some(d) = &st.dgpu_disable {
                sysfs.push((d.clone(), "1".into()));
            }
        }
        GfxMode::Standard => {
            revive(&mut sysfs);
            if let Some(m) = &st.mux {
                if mux_now == Some(0) {
                    sysfs.push((m.clone(), "1".into()));
                }
            }
            if st.cardwire {
                post.push(cardwire("hybrid"));
            }
        }
        GfxMode::Ultimate => {
            if dgpu_off {
                if let Some(d) = &st.dgpu_disable {
                    sysfs.push((d.clone(), "0".into()));
                }
            }
            if let Some(m) = &st.mux {
                sysfs.push((m.clone(), "0".into()));
            }
            if st.cardwire {
                // Unblock first, or Cardwire would block the GPU driving the screen at boot.
                post.push(cardwire("hybrid"));
            }
        }
    }
    let reboot = mode == GfxMode::Ultimate || mux_now == Some(0);
    // The same switches through asusctl (no password), in the same order; the direct
    // firmware write above is the fallback. A PCI rescan has no asusctl form, so that
    // case goes straight to the direct write.
    let groups = sysfs
        .iter()
        .filter_map(|(p, v)| {
            let name = attr_name(p).filter(|_| p.starts_with("/sys/class") || p.starts_with("/sys/devices"))?;
            Some(vec![argv(&["asusctl", "armoury", "set", name.as_str(), v.as_str()])])
        })
        .collect::<Vec<_>>();
    Action {
        label: format!("{GFX_LABEL} {}", mode.title()),
        groups: if groups.len() == sysfs.len() { groups } else { Vec::new() },
        sysfs,
        needs_reboot: reboot,
        post,
    }
}

/// Cardwire's current mode ("integrated", "hybrid"...), when its daemon answers.
pub fn cardwire_mode() -> Option<String> {
    let out = run(&argv(&["cardwire", "get"])).ok()?;
    let line = out.lines().find(|l| l.starts_with("Current Mode:"))?;
    Some(line.trim_start_matches("Current Mode:").trim().to_lowercase())
}

/// The firmware attribute a sysfs path belongs to:
/// `.../asus-armoury/attributes/<name>/current_value` or `.../asus-nb-wmi/<name>`.
fn attr_name(p: &Path) -> Option<String> {
    let file = p.file_name()?.to_str()?;
    let name = if file == "current_value" { p.parent()?.file_name()?.to_str()? } else { file };
    Some(if name == "panel_od" { "panel_overdrive".into() } else { name.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output of `asusctl fan-curve --mod-profile Quiet` (asusctl 6.5.0, GA402RJ).
    const QUIET: &str = "Fan curves for Quiet\n\n[\n    (\n        fan: CPU,\n        pwm: (7, 30, 30, 56, 56, 86, 86, 104),\n        temp: (60, 65, 65, 70, 70, 75, 75, 98),\n        enabled: false,\n    ),\n    (\n        fan: GPU,\n        pwm: (20, 43, 43, 73, 73, 96, 96, 114),\n        temp: (60, 65, 65, 70, 70, 75, 75, 98),\n        enabled: false,\n    ),\n]\n";

    #[test]
    fn reads_asusctl_fan_curves() {
        let (cpu, gpu) = parse_fan_curves(QUIET).expect("parsed");
        assert_eq!(cpu[0], (60, 3));
        assert_eq!(cpu[7], (98, 41));
        assert_eq!(gpu[0], (60, 8));
        assert_eq!(gpu[7], (98, 45));
    }

    fn st(dgpu_disabled: i64, mux_now: i64, cardwire: bool) -> GfxState {
        let a = "/sys/class/firmware-attributes/asus-armoury/attributes";
        GfxState {
            dgpu_disable: Some(format!("{a}/dgpu_disable/current_value").into()),
            dgpu_disabled: Some(dgpu_disabled),
            mux: Some(format!("{a}/gpu_mux_mode/current_value").into()),
            mux_now: Some(mux_now),
            cardwire,
        }
    }
    fn cmds(a: &Action) -> Vec<String> {
        a.groups.iter().map(|g| g[0].join(" ")).chain(a.post.iter().map(|c| c.join(" "))).collect()
    }

    #[test]
    fn graphics_switch() {
        // Standard → Ultimate: MUX through asusctl, Cardwire unblocks first.
        let u = set_gfx(GfxMode::Ultimate, &st(0, 1, true));
        assert_eq!(cmds(&u), ["asusctl armoury set gpu_mux_mode 0", "cardwire set hybrid"]);
        assert!(u.needs_reboot);
        // Eco from Ultimate only hands the screen back.
        let e = set_gfx(GfxMode::Eco, &st(0, 0, true));
        assert_eq!(cmds(&e), ["asusctl armoury set gpu_mux_mode 1"]);
        // Eco and back with Cardwire: no firmware switch at all.
        assert_eq!(cmds(&set_gfx(GfxMode::Eco, &st(0, 1, true))), ["cardwire set integrated"]);
        let back = set_gfx(GfxMode::Standard, &st(0, 1, true));
        assert_eq!(cmds(&back), ["cardwire set hybrid"]);
        assert!(back.sysfs.is_empty() && !back.needs_reboot);
        // Coming back from the firmware's "dGPU off": clear it and rescan the PCI bus.
        let revive = set_gfx(GfxMode::Standard, &st(1, 1, true));
        assert!(revive.groups.is_empty());
        let paths: Vec<_> = revive.sysfs.iter().map(|(p, v)| format!("{}={v}", p.display())).collect();
        assert!(paths[0].ends_with("dgpu_disable/current_value=0"));
        assert_eq!(paths[1], "/sys/bus/pci/rescan=1");
        assert_eq!(revive.post[0].join(" "), "cardwire set hybrid");
    }
}
