//! Read-only view of the laptop through sysfs / procfs.
//!
//! Everything here works without root and without asusd running; it is what the
//! dashboard shows. Changes go through `ctl.rs` (asusctl first, privileged sysfs
//! write as a fallback).

use std::fs;
use std::path::{Path, PathBuf};

pub const PLATFORM_PROFILE: &str = "/sys/firmware/acpi/platform_profile";
pub const PLATFORM_PROFILE_CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";
pub const ASUS_WMI: &str = "/sys/devices/platform/asus-nb-wmi";
pub const ARMOURY: &str = "/sys/class/firmware-attributes/asus-armoury/attributes";
pub const KBD_LED: &str = "/sys/class/leds/asus::kbd_backlight";

pub fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn read_num<T: std::str::FromStr>(path: impl AsRef<Path>) -> Option<T> {
    read(path)?.parse().ok()
}

/// One BIOS / firmware attribute exposed by the asus-armoury driver
/// (or the older asus-nb-wmi files).
#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct Attr {
    pub name: String,
    pub path: PathBuf,
    pub current: Option<i64>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    /// The firmware's stock value (asus-armoury's default_value).
    pub default: Option<i64>,
    pub possible: Vec<i64>,
    pub display: String,
}

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub card: String,
    pub discrete: bool,
    pub awake: bool,
    pub temp_c: Option<f32>,
    pub busy_pct: Option<u32>,
    pub vram_used_mb: Option<u64>,
    pub vram_total_mb: Option<u64>,
    /// Power draw (amdgpu "PPT"). On an AMD APU the integrated GPU's figure is the whole
    /// processor package (CPU cores + graphics).
    pub power_w: Option<f32>,
    /// Current graphics clock (MHz).
    pub clock_mhz: Option<u32>,
    pub device_id: String,
}

/// 8 points: (temperature °C, duty 0-100 %)
pub type Curve = [(u8, u8); 8];

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub model: String,
    pub cpu_name: String,
    pub kernel: String,
    pub profile: Option<String>,
    pub profile_choices: Vec<String>,
    pub charge_limit: Option<u8>,
    pub charge_limit_path: Option<PathBuf>,
    pub battery_pct: Option<u8>,
    pub battery_status: String,
    pub battery_power_w: Option<f32>,
    /// Energy in the battery now, and when full (Wh).
    pub battery_energy_wh: Option<f32>,
    pub battery_full_wh: Option<f32>,
    pub battery_health_pct: Option<f32>,
    pub battery_cycles: Option<u32>,
    pub on_ac: Option<bool>,
    pub cpu_temp_c: Option<f32>,
    pub cpu_mhz: Option<u32>,
    pub cpu_load_pct: Option<f32>,
    pub mem_used_gb: Option<f32>,
    pub mem_total_gb: Option<f32>,
    pub fans: Vec<(String, u32)>,
    pub gpus: Vec<Gpu>,
    pub kbd_brightness: Option<u8>,
    pub kbd_max: Option<u8>,
    pub attrs: Vec<Attr>,
    /// The firmware says a change (GPU MUX, dGPU on/off...) waits for a restart.
    pub pending_reboot: bool,
    /// Cardwire's mode ("integrated", "hybrid"...) when its daemon runs.
    pub cardwire_mode: Option<String>,
    /// Mounted drives with their free space (refreshed every 15 s).
    pub disks: Vec<Disk>,
    /// Curves currently loaded in the firmware, read from asus_custom_fan_curve.
    pub cpu_curve: Option<Curve>,
    pub gpu_curve: Option<Curve>,
    pub curve_hwmon: Option<PathBuf>,
    pub cpu_curve_enabled: Option<bool>,
    pub gpu_curve_enabled: Option<bool>,
    /// A dot-matrix lid display (USB 0b05:193b / 0b05:19b3) is present.
    pub has_lid_matrix: bool,
    /// asusd's own power limits for each mode, plugged in and on battery.
    pub tunings: Vec<Tuning>,
    /// The graphics mode chosen to start after the next restart (None: nothing waiting).
    pub gfx_pending: Option<crate::ctl::GfxMode>,
}

/// Notices the fans held at (nearly) full speed while nothing is warm, for a minute and a
/// half. Seen on the G14 after leaving Ultimate: the laptop's fan controller keeps a stale
/// state through a restart, and no mode, fan curve or sleep releases it.
#[derive(Default)]
pub struct StuckFans {
    since: Option<std::time::Instant>,
}

impl StuckFans {
    pub const FAST_RPM: u32 = 5000;

    /// Feed every new reading; true while the fans have been stuck long enough to say so.
    pub fn update(&mut self, s: &Snapshot) -> bool {
        let fast = !s.fans.is_empty() && s.fans.iter().all(|(_, rpm)| *rpm >= Self::FAST_RPM);
        let cool = s.cpu_temp_c.is_some_and(|t| t <= 72.0)
            && s.gpus.iter().all(|g| !g.awake || g.temp_c.is_none_or(|t| t <= 66.0));
        if !(fast && cool) {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert_with(std::time::Instant::now);
        // ARCHERYCRATE_STUCK_SECS shortens the wait for checks and screenshots.
        let wait = std::env::var("ARCHERYCRATE_STUCK_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
        since.elapsed() >= std::time::Duration::from_secs(wait)
    }
}

/// What to tell the person when the fans are stuck (the window's banner and the tray's notification).
pub const STUCK_FANS_TEXT: &str = "The fans are running at full speed although the laptop is cool. This can happen \
after switching graphics out of Ultimate: the laptop's fan controller keeps an old state through a restart, and \
changing modes doesn't release it. A full shut down should clear it: shut down (not restart), wait about 10 seconds, \
then turn the laptop back on.";

/// One mode's power limits as asusd keeps them (`ac_profile_tunings` / `dc_profile_tunings`
/// in /etc/asusd/asusd.ron). When `enabled`, asusd puts `values` in place whenever that mode
/// starts; otherwise the mode uses the laptop's stock limits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tuning {
    pub on_ac: bool,
    /// asusctl's name: Quiet, Balanced or Performance.
    pub profile: String,
    pub enabled: bool,
    /// (attribute name such as ppt_apu_sppt, watts)
    pub values: Vec<(String, i64)>,
}

/// Power-limit attributes (the sliders on the Performance page).
pub fn is_power_limit(name: &str) -> bool {
    name.starts_with("ppt_") || name.starts_with("nv_")
}

impl Snapshot {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }

    pub fn power_limits(&self) -> Vec<&Attr> {
        self.attrs.iter().filter(|a| is_power_limit(&a.name)).collect()
    }

    /// The active mode's power limits for the current power source, as asusd keeps them.
    pub fn tuning_now(&self) -> Option<&Tuning> {
        let profile = crate::ctl::asusctl_profile(self.profile.as_deref()?);
        let on_ac = self.on_ac.unwrap_or(true);
        self.tunings.iter().find(|t| t.profile == profile && t.on_ac == on_ac)
    }

    /// The value a power-limit slider shows for the active mode. With limits of its own, the
    /// ones asusd keeps for it. Without, the stock setting: the firmware then uses the mode's
    /// own limits, and the attribute's "current" value is only the last number written (it
    /// stays behind after leaving a mode that had its own).
    pub fn power_limit_shown(&self, a: &Attr) -> Option<i64> {
        match self.tuning_now().filter(|t| t.enabled) {
            Some(t) => t.values.iter().find(|(n, _)| *n == a.name).map(|(_, v)| *v).or(a.current),
            None => a.default.or(a.current),
        }
    }

    /// The graphics mode, from the MUX, Cardwire and the firmware's dGPU switch.
    pub fn gfx_mode(&self) -> crate::ctl::GfxMode {
        use crate::ctl::GfxMode;
        if self.attr("gpu_mux_mode").and_then(|a| a.current) == Some(0) {
            GfxMode::Ultimate
        } else if self.cardwire_mode.as_deref() == Some("integrated")
            || self.attr("dgpu_disable").and_then(|a| a.current) == Some(1)
        {
            GfxMode::Eco
        } else {
            GfxMode::Standard
        }
    }

    pub fn gfx_state(&self) -> crate::ctl::GfxState {
        let d = self.attr("dgpu_disable");
        let m = self.attr("gpu_mux_mode");
        crate::ctl::GfxState {
            dgpu_disable: d.map(|a| a.path.clone()),
            dgpu_disabled: d.and_then(|a| a.current),
            mux: m.map(|a| a.path.clone()),
            mux_now: m.and_then(|a| a.current),
            cardwire: self.cardwire_mode.is_some(),
            pending: self.gfx_pending,
        }
    }

    pub fn dgpu(&self) -> Option<&Gpu> {
        self.gpus.iter().find(|g| g.discrete)
    }

    pub fn igpu(&self) -> Option<&Gpu> {
        self.gpus.iter().find(|g| !g.discrete)
    }
}

/// CPU usage needs two samples; keep the previous one between refreshes.
#[derive(Default)]
pub struct Sampler {
    prev_cpu: Option<(u64, u64)>,
    static_info: Option<(String, String, String)>,
    has_cardwire: Option<bool>,
    cardwire_cache: Option<String>,
    cardwire_at: Option<std::time::Instant>,
    disks_cache: Vec<Disk>,
    disks_at: Option<std::time::Instant>,
}

impl Sampler {
    /// Ask Cardwire again on the next reading (after a graphics change).
    pub fn forget_cardwire(&mut self) {
        self.cardwire_at = None;
    }
}

impl Sampler {
    pub fn sample(&mut self) -> Snapshot {
        let mut s = Snapshot::default();

        let (model, cpu, kernel) = self
            .static_info
            .get_or_insert_with(|| {
                let model = read("/sys/class/dmi/id/product_name").unwrap_or_else(|| "Unknown laptop".into());
                let cpu = fs::read_to_string("/proc/cpuinfo")
                    .ok()
                    .and_then(|c| {
                        c.lines()
                            .find(|l| l.starts_with("model name"))
                            .and_then(|l| l.split(':').nth(1))
                            .map(|v| v.trim().to_string())
                    })
                    .unwrap_or_default();
                let kernel = read("/proc/sys/kernel/osrelease").unwrap_or_default();
                (model, cpu, kernel)
            })
            .clone();
        s.model = model;
        s.cpu_name = cpu;
        s.kernel = kernel;

        // Performance profile
        // /sys/firmware isn't visible inside Flatpak; newer kernels also offer a class device.
        let class_dir = fs::read_dir("/sys/class/platform-profile")
            .ok()
            .and_then(|mut d| d.next())
            .and_then(|e| e.ok())
            .map(|e| e.path());
        s.profile = read(PLATFORM_PROFILE).or_else(|| class_dir.as_ref().and_then(|d| read(d.join("profile"))));
        s.profile_choices = read(PLATFORM_PROFILE_CHOICES)
            .or_else(|| class_dir.as_ref().and_then(|d| read(d.join("choices"))))
            .map(|c| c.split_whitespace().map(String::from).collect())
            .unwrap_or_default();

        // Battery + AC
        if let Ok(dir) = fs::read_dir("/sys/class/power_supply") {
            // The laptop's own battery only: wireless keyboards, mice and headsets also show up
            // as "Battery" supplies (scope "Device", e.g. hidpp_battery_0). BAT* names first.
            let mut entries: Vec<PathBuf> = dir.flatten().map(|e| e.path()).collect();
            entries.sort_by_key(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                (!n.starts_with("BAT"), n)
            });
            for p in entries {
                let kind = read(p.join("type")).unwrap_or_default();
                let device = read(p.join("scope")).as_deref() == Some("Device");
                if kind == "Battery" && !device && s.battery_pct.is_none() {
                    s.battery_pct = read_num(p.join("capacity"));
                    s.battery_status = read(p.join("status")).unwrap_or_default();
                    s.battery_power_w = read_num::<f64>(p.join("power_now")).map(|v| (v / 1e6) as f32);
                    s.battery_cycles = read_num(p.join("cycle_count"));
                    // Energy in the battery now and when full (Wh), for "time left". Batteries
                    // that count charge (µAh) instead are converted with the voltage.
                    let volts = read_num::<f64>(p.join("voltage_now")).map(|v| v / 1e6);
                    let wh = |energy: &str, charge: &str| {
                        read_num::<f64>(p.join(energy))
                            .map(|v| v / 1e6)
                            .or_else(|| Some(read_num::<f64>(p.join(charge))? / 1e6 * volts?))
                            .map(|v| v as f32)
                    };
                    s.battery_energy_wh = wh("energy_now", "charge_now");
                    s.battery_full_wh = wh("energy_full", "charge_full");
                    let full = read_num::<f64>(p.join("energy_full")).or_else(|| read_num(p.join("charge_full")));
                    let design = read_num::<f64>(p.join("energy_full_design"))
                        .or_else(|| read_num(p.join("charge_full_design")));
                    if let (Some(f), Some(d)) = (full, design) {
                        if d > 0.0 {
                            s.battery_health_pct = Some((f / d * 100.0) as f32);
                        }
                    }
                    let lim = p.join("charge_control_end_threshold");
                    if lim.exists() {
                        s.charge_limit = read_num(&lim);
                        s.charge_limit_path = Some(lim);
                    }
                } else if kind == "Mains" {
                    s.on_ac = read_num::<u8>(p.join("online")).map(|v| v == 1);
                }
            }
        }

        // Temperatures, fans
        if let Ok(dir) = fs::read_dir("/sys/class/hwmon") {
            for e in dir.flatten() {
                let p = e.path();
                let name = read(p.join("name")).unwrap_or_default();
                match name.as_str() {
                    "k10temp" | "coretemp" | "zenpower" => {
                        s.cpu_temp_c = read_num::<f32>(p.join("temp1_input")).map(|v| v / 1000.0);
                    }
                    "asus" => {
                        for i in 1..=3 {
                            if let Some(rpm) = read_num::<u32>(p.join(format!("fan{i}_input"))) {
                                let label = read(p.join(format!("fan{i}_label"))).unwrap_or_else(|| format!("fan{i}"));
                                s.fans.push((label, rpm));
                            }
                        }
                    }
                    "asus_custom_fan_curve" => {
                        s.curve_hwmon = Some(p.clone());
                        s.cpu_curve = read_curve(&p, 1);
                        s.gpu_curve = read_curve(&p, 2);
                        s.cpu_curve_enabled = read_num::<u8>(p.join("pwm1_enable")).map(|v| v == 1);
                        s.gpu_curve_enabled = read_num::<u8>(p.join("pwm2_enable")).map(|v| v == 1);
                    }
                    _ => {}
                }
            }
        }
        if s.cpu_temp_c.is_none() {
            // acpi thermal zone fallback
            s.cpu_temp_c = read_num::<f32>("/sys/class/thermal/thermal_zone0/temp").map(|v| v / 1000.0);
        }

        // CPU frequency (average of cores) and load
        let mut sum = 0u64;
        let mut n = 0u64;
        for i in 0..256 {
            match read_num::<u64>(format!("/sys/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq")) {
                Some(f) => {
                    sum += f;
                    n += 1;
                }
                None => {
                    if i > 0 && !Path::new(&format!("/sys/devices/system/cpu/cpu{i}")).exists() {
                        break;
                    }
                }
            }
        }
        if n > 0 {
            s.cpu_mhz = Some((sum / n / 1000) as u32);
        }
        if let Some(line) = fs::read_to_string("/proc/stat").ok().and_then(|t| t.lines().next().map(String::from)) {
            let v: Vec<u64> = line.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 4 {
                let idle = v[3] + v.get(4).copied().unwrap_or(0);
                let total: u64 = v.iter().sum();
                if let Some((pi, pt)) = self.prev_cpu {
                    let dt = total.saturating_sub(pt);
                    if dt > 0 {
                        s.cpu_load_pct = Some(100.0 * (1.0 - (idle.saturating_sub(pi)) as f32 / dt as f32));
                    }
                }
                self.prev_cpu = Some((idle, total));
            }
        }

        // Memory
        if let Ok(m) = fs::read_to_string("/proc/meminfo") {
            let get = |k: &str| {
                m.lines()
                    .find(|l| l.starts_with(k))
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|v| v.parse::<f32>().ok())
            };
            if let (Some(t), Some(a)) = (get("MemTotal:"), get("MemAvailable:")) {
                s.mem_total_gb = Some(t / 1048576.0);
                s.mem_used_gb = Some((t - a) / 1048576.0);
            }
        }

        // GPUs
        if let Ok(dir) = fs::read_dir("/sys/class/drm") {
            let mut cards: Vec<_> = dir
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with("card") && !n.contains('-'))
                .collect();
            cards.sort();
            for card in cards {
                let dev = PathBuf::from("/sys/class/drm").join(&card).join("device");
                let runtime = read(dev.join("power/runtime_status")).unwrap_or_else(|| "active".into());
                let awake = runtime == "active";
                let mut g = Gpu {
                    card: card.clone(),
                    discrete: is_discrete(&dev),
                    awake,
                    device_id: read(dev.join("device")).unwrap_or_default(),
                    ..Default::default()
                };
                // Only touch the counters while the GPU is awake so we never wake a sleeping dGPU.
                if awake {
                    g.busy_pct = read_num(dev.join("gpu_busy_percent"));
                    g.vram_used_mb = read_num::<u64>(dev.join("mem_info_vram_used")).map(|v| v / 1048576);
                    g.vram_total_mb = read_num::<u64>(dev.join("mem_info_vram_total")).map(|v| v / 1048576);
                    if let Ok(hw) = fs::read_dir(dev.join("hwmon")) {
                        for h in hw.flatten() {
                            let hp = h.path();
                            g.temp_c = read_num::<f32>(hp.join("temp1_input")).map(|v| v / 1000.0);
                            g.power_w = read_num::<f32>(hp.join("power1_average"))
                                .or_else(|| read_num::<f32>(hp.join("power1_input")))
                                .map(|v| v / 1e6);
                            // The shader clock: freq1 labelled "sclk", in Hz.
                            if read(hp.join("freq1_label")).as_deref() == Some("sclk") {
                                g.clock_mhz = read_num::<u64>(hp.join("freq1_input")).map(|hz| (hz / 1_000_000) as u32);
                            }
                        }
                    }
                }
                s.gpus.push(g);
            }
        }

        // Keyboard backlight brightness
        s.kbd_brightness = read_num(format!("{KBD_LED}/brightness"));
        s.kbd_max = read_num(format!("{KBD_LED}/max_brightness"));

        // Firmware attributes
        s.attrs = read_attrs();
        s.tunings = read_tunings();
        s.pending_reboot = read_num::<u8>(format!("{ARMOURY}/pending_reboot")) == Some(1);
        if self.disks_at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(15)) {
            self.disks_cache = read_disks();
            self.disks_at = Some(std::time::Instant::now());
        }
        s.disks = self.disks_cache.clone();
        if *self.has_cardwire.get_or_insert_with(|| {
            crate::ctl::run_host(&["sh", "-c", "command -v cardwire"]).is_ok()
        }) {
            // Asking Cardwire costs a log entry on its side each time: at most every 5 s.
            let stale = self.cardwire_at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(5));
            if stale {
                self.cardwire_cache = crate::ctl::cardwire_mode();
                self.cardwire_at = Some(std::time::Instant::now());
            }
            s.cardwire_mode = self.cardwire_cache.clone();
        }
        // A graphics switch waiting for the restart (the firmware only takes it then).
        s.gfx_pending = crate::ctl::read_pending_gfx().filter(|p| *p != s.gfx_mode());

        // Lid matrix display (USB)
        if let Ok(dir) = fs::read_dir("/sys/bus/usb/devices") {
            s.has_lid_matrix = dir.flatten().any(|e| {
                let p = e.path();
                read(p.join("idVendor")).as_deref() == Some("0b05")
                    && matches!(read(p.join("idProduct")).as_deref(), Some("193b") | Some("19b3"))
            });
        }

        s
    }
}

/// Whether a GPU is the discrete one.
///
/// `boot_vga` can't be used for this: it marks whichever GPU drives the screen, and in
/// Ultimate (MUX) mode that's the discrete GPU. Instead:
/// * NVIDIA chips in laptops are always discrete;
/// * amdgpu reports the maker of its video memory chips only for a card with its own VRAM;
/// * a discrete GPU sits behind its own PCIe switch, so it's deeper in the PCI tree than
///   an integrated one (Intel at 00:02.0, AMD APUs behind the internal 00:08.x bridge).
/// Wake the discrete GPU for a moment so it looks at its ports again.
///
/// After Cardwire unblocks it (Eco → Standard), the discrete GPU stays in its deepest sleep
/// (D3cold) and doesn't notice a monitor plugged into the HDMI port, which it drives on the
/// G14. Opening its render node resumes it; on resume the driver re-detects connected
/// monitors. The node is held open briefly, then released so the GPU can sleep again if
/// nothing needs it. Needs no password (render nodes are open to the signed-in person).
pub fn wake_dgpu() -> Result<String, String> {
    let dir = fs::read_dir("/sys/class/drm").map_err(|e| e.to_string())?;
    let node = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("renderD"))
        .find(|n| is_discrete(&Path::new("/sys/class/drm").join(n).join("device")))
        .ok_or("no discrete GPU found (still switched off?)")?;
    let path = Path::new("/dev/dri").join(&node);
    let file = fs::File::open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    // Long enough for the GPU to resume and probe its outputs (~0.3 s here).
    std::thread::sleep(std::time::Duration::from_millis(1500));
    drop(file);
    Ok(format!("woke the discrete GPU ({node}) so it sees connected monitors"))
}

/// The PCI vendor and device id of the discrete GPU when it drives the built-in screen
/// (Ultimate mode, after the restart), so the window can draw there and leave the
/// integrated GPU idle. `None` in every other mode.
pub fn dgpu_drives_builtin_screen() -> Option<(u32, u32)> {
    let hex = |p: PathBuf| read(p).and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok());
    for e in fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // Connectors are named like card1-eDP-1.
        let Some((card, conn)) = name.split_once('-') else { continue };
        if !conn.starts_with("eDP") || read(e.path().join("status")).as_deref() != Some("connected") {
            continue;
        }
        let dev = Path::new("/sys/class/drm").join(card).join("device");
        if is_discrete(&dev) {
            return Some((hex(dev.join("vendor"))?, hex(dev.join("device"))?));
        }
    }
    None
}

/// An external monitor connected to the discrete GPU's own outputs (e.g. "HDMI"): while one
/// is, the discrete GPU has to stay on to drive it.
pub fn dgpu_external_display() -> Option<String> {
    for e in fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some((card, conn)) = name.split_once('-') else { continue };
        if conn.starts_with("eDP") || conn.starts_with("Writeback") || read(e.path().join("status")).as_deref() != Some("connected") {
            continue;
        }
        if is_discrete(&Path::new("/sys/class/drm").join(card).join("device")) {
            // HDMI-A-1 → HDMI, DP-1 → DisplayPort
            let kind = conn.split('-').next().unwrap_or(conn);
            return Some(if kind == "DP" { "DisplayPort".into() } else { kind.to_string() });
        }
    }
    None
}

pub fn is_discrete(dev: &Path) -> bool {
    if read(dev.join("vendor")).as_deref() == Some("0x10de") {
        return true;
    }
    if read(dev.join("mem_info_vram_vendor")).is_some_and(|v| !v.is_empty()) {
        return true;
    }
    let depth = fs::canonicalize(dev)
        .map(|p| {
            p.components()
                .filter(|c| {
                    let s = c.as_os_str().to_string_lossy();
                    s.len() == 12 && s.chars().nth(4) == Some(':') && s.chars().nth(7) == Some(':')
                })
                .count()
        })
        .unwrap_or(0);
    depth >= 3
}

/// A mounted drive and its space.
#[derive(Clone, Debug, Default)]
pub struct Disk {
    /// A friendly name: "System drive", or the volume label / folder name.
    pub name: String,
    pub mount: String,

    pub size: u64,
    pub avail: u64,
}

impl Disk {
    pub fn used_frac(&self) -> f32 {
        if self.size == 0 { 0.0 } else { 1.0 - self.avail as f32 / self.size as f32 }
    }
}

/// Real drives from `df` (block devices only), one row per device: btrfs and similar mount
/// the same device at several places (/, /home, /var/log …), which is one drive to a person.
/// Boot/EFI partitions and read-only images (snaps, loop devices) are left out.
fn read_disks() -> Vec<Disk> {
    let Ok(out) = crate::ctl::run_host(&["df", "-B1", "--output=source,fstype,size,avail,target"]) else {
        return Vec::new();
    };
    let mut disks: Vec<(String, Disk)> = Vec::new();
    for line in out.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 5 {
            continue;
        }
        let (src, fstype) = (f[0], f[1]);
        let target = f[4..].join(" ");
        if !src.starts_with("/dev/") || src.starts_with("/dev/loop") {
            continue;
        }
        if matches!(fstype, "squashfs" | "iso9660" | "overlay" | "tmpfs" | "devtmpfs")
            || target.starts_with("/boot")
            || target.starts_with("/efi")
        {
            continue;
        }
        let (Ok(size), Ok(avail)) = (f[2].parse::<u64>(), f[3].parse::<u64>()) else { continue };
        if size < 512 * 1024 * 1024 {
            continue; // tiny system partitions
        }
        let d = Disk { name: disk_name(&target), mount: target, size, avail };
        match disks.iter_mut().find(|(s, _)| s == src) {
            // Same device again: keep the shortest mount point (/ before /home).
            Some((_, old)) => {
                if d.mount.len() < old.mount.len() {
                    *old = d;
                }
            }
            None => disks.push((src.to_string(), d)),
        }
    }
    let mut v: Vec<Disk> = disks.into_iter().map(|(_, d)| d).collect();
    v.sort_by_key(|d| (d.mount != "/", d.mount.clone()));
    v
}

fn disk_name(mount: &str) -> String {
    match mount {
        "/" => "System drive".into(),
        "/home" => "Home".into(),
        m => m.rsplit('/').find(|s| !s.is_empty()).unwrap_or(m).replace("\\x20", " "),
    }
}

/// "1.4 TB", "512 GB", "900 MB".
pub fn human_bytes(b: u64) -> String {
    let gb = b as f64 / 1e9;
    if gb >= 1000.0 {
        format!("{:.1} TB", gb / 1000.0)
    } else if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

fn read_curve(hwmon: &Path, pwm: u8) -> Option<Curve> {
    let mut c: Curve = [(0, 0); 8];
    for (i, slot) in c.iter_mut().enumerate() {
        let t: u8 = read_num(hwmon.join(format!("pwm{pwm}_auto_point{}_temp", i + 1)))?;
        let d: u32 = read_num(hwmon.join(format!("pwm{pwm}_auto_point{}_pwm", i + 1)))?;
        *slot = (t, ((d * 100 + 127) / 255).min(100) as u8);
    }
    Some(c)
}

fn read_tunings() -> Vec<Tuning> {
    read("/etc/asusd/asusd.ron").map(|t| parse_tunings(&t)).unwrap_or_default()
}

/// Reads the `ac_profile_tunings` / `dc_profile_tunings` blocks of asusd.ron.
pub fn parse_tunings(text: &str) -> Vec<Tuning> {
    // PptApuSppt → ppt_apu_sppt, PptPl1Spl → ppt_pl1_spl
    let snake = |s: &str| {
        let mut o = String::new();
        for (i, ch) in s.chars().enumerate() {
            if ch.is_ascii_uppercase() && i > 0 {
                o.push('_');
            }
            o.push(ch.to_ascii_lowercase());
        }
        o
    };
    let mut out: Vec<Tuning> = Vec::new();
    let mut side: Option<bool> = None;
    for line in text.lines() {
        let l = line.trim();
        let indent = line.len() - line.trim_start().len();
        // Top-level fields sit at the shallowest indent inside "(": a new one starts or ends a block.
        if indent <= 4 && !l.is_empty() && l != ")" {
            side = if l.starts_with("ac_profile_tunings") {
                Some(true)
            } else if l.starts_with("dc_profile_tunings") {
                Some(false)
            } else {
                None
            };
            continue;
        }
        let Some(on_ac) = side else { continue };
        let Some((key, val)) = l.split_once(':') else { continue };
        let (key, val) = (key.trim(), val.trim().trim_end_matches(','));
        match key {
            "Quiet" | "Balanced" | "Performance" | "LowPower" | "Custom" if val == "(" => {
                out.push(Tuning { on_ac, profile: key.into(), ..Default::default() });
            }
            "enabled" => {
                if let Some(t) = out.last_mut() {
                    t.enabled = val == "true";
                }
            }
            k if k.starts_with("Ppt") || k.starts_with("Nv") => {
                if let (Some(t), Ok(v)) = (out.last_mut(), val.parse()) {
                    t.values.push((snake(k), v));
                }
            }
            _ => {}
        }
    }
    out
}

fn read_attrs() -> Vec<Attr> {
    let mut out = Vec::new();
    if let Ok(dir) = fs::read_dir(ARMOURY) {
        for e in dir.flatten() {
            let p = e.path();
            let cur = p.join("current_value");
            if !cur.exists() {
                continue;
            }
            let possible = read(p.join("possible_values"))
                .map(|v| v.split(';').filter_map(|x| x.trim().parse().ok()).collect())
                .unwrap_or_default();
            out.push(Attr {
                name: e.file_name().to_string_lossy().to_string(),
                current: read_num(&cur),
                min: read_num(p.join("min_value")),
                max: read_num(p.join("max_value")),
                default: read_num(p.join("default_value")),
                possible,
                display: read(p.join("display_name")).unwrap_or_default(),
                path: cur,
            });
        }
    } else {
        // Older kernels: the same knobs live directly under asus-nb-wmi.
        for (name, poss) in [
            ("boot_sound", vec![0, 1]),
            ("panel_od", vec![0, 1]),
            ("gpu_mux_mode", vec![0, 1]),
            ("dgpu_disable", vec![0, 1]),
        ] {
            let path = PathBuf::from(ASUS_WMI).join(name);
            if path.exists() {
                out.push(Attr {
                    name: if name == "panel_od" { "panel_overdrive".into() } else { name.into() },
                    current: read_num(&path),
                    possible: poss,
                    path,
                    ..Default::default()
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_asusd_power_limits() {
        let text = "(\n    charge_control_end_threshold: 80,\n    ac_profile_tunings: {\n        Balanced: (\n            enabled: true,\n            group: {\n                PptApuSppt: 60,\n                PptPl1Spl: 45,\n            },\n        ),\n        Quiet: (\n            enabled: false,\n            group: {},\n        ),\n    },\n    dc_profile_tunings: {\n        Quiet: (\n            enabled: true,\n            group: {\n                PptPlatformSppt: 40,\n            },\n        ),\n    },\n    armoury_settings: {\n        PptApuSppt: 99,\n        BootSound: 0,\n    },\n)";
        let t = parse_tunings(text);
        assert_eq!(t.len(), 3);
        assert_eq!(
            t[0],
            Tuning {
                on_ac: true,
                profile: "Balanced".into(),
                enabled: true,
                values: vec![("ppt_apu_sppt".into(), 60), ("ppt_pl1_spl".into(), 45)],
            }
        );
        assert!(!t[1].enabled && t[1].values.is_empty());
        assert_eq!((t[2].on_ac, t[2].enabled, t[2].values.clone()), (false, true, vec![("ppt_platform_sppt".into(), 40)]));
        // The real file on this laptop, when there is one, must read without surprises.
        if let Some(real) = read("/etc/asusd/asusd.ron") {
            assert!(parse_tunings(&real).iter().all(|t| t.values.iter().all(|(n, _)| is_power_limit(n))));
        }
    }
}
