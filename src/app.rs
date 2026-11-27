//! The ArcheryCrate window.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Ui, ViewportCommand, vec2};

use crate::ctl::{self, Action, AuraMode, GfxMode, Msg, Outcome, Tools};
use crate::hw::{self, Sampler, Snapshot};
use crate::settings::{self, Settings};
use crate::setup;
use crate::tray::{self, TrayCmd};
use crate::widgets::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Summary,
    Performance,
    Fans,
    Battery,
    Lighting,
    Graphics,
    System,
    Activity,
}

impl Page {
    /// Sidebar order, top to bottom.
    const ALL: [Page; 9] = [
        Page::Home,
        Page::Performance,
        Page::Graphics,
        Page::Fans,
        Page::Battery,
        Page::Lighting,
        Page::Summary,
        Page::System,
        Page::Activity,
    ];
    fn title(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Summary => "Summary",
            Page::Performance => "Performance",
            Page::Fans => "Fans",
            Page::Battery => "Battery",
            Page::Lighting => "Lighting",
            Page::Graphics => "Graphics",
            Page::System => "Settings",
            Page::Activity => "Activity",
        }
    }
    /// Name of the drawn icon (see `widgets::neon_icon`).
    fn key(self) -> &'static str {
        match self {
            Page::Home => "home",
            Page::Summary => "summary",
            Page::Performance => "performance",
            Page::Fans => "fans",
            Page::Battery => "battery",
            Page::Lighting => "lighting",
            Page::Graphics => "graphics",
            Page::System => "settings",
            Page::Activity => "activity",
        }
    }
}

pub fn profile_title(p: &str) -> &'static str {
    match p {
        "quiet" | "low-power" => "Silent",
        "balanced" => "Balanced",
        "performance" => "Turbo",
        "custom" => "Custom",
        _ => "Unknown",
    }
}

fn profile_color(p: &str) -> Color32 {
    match p {
        "quiet" | "low-power" => CYAN(),
        "performance" => ACCENT(),
        _ => GREEN(),
    }
}

pub fn attr_title(name: &str) -> &'static str {
    match name {
        "panel_overdrive" => "Panel overdrive",
        "boot_sound" => "Boot sound",
        "ppt_apu_sppt" => "Processor power limit",
        "ppt_platform_sppt" => "Combined power limit",
        "ppt_pl1_spl" => "Processor steady limit",
        "ppt_pl2_sppt" => "Processor short-burst limit",
        "ppt_fppt" | "ppt_pl3_fppt" => "Processor quick-burst limit",
        "nv_dynamic_boost" => "Graphics boost",
        "nv_temp_target" => "Graphics temperature target",
        "nv_tgp" => "Graphics card power limit",
        "dgpu_disable" => "dGPU disabled",
        "gpu_mux_mode" => "GPU MUX",
        "charge_mode" => "Charge mode",
        "mcu_powersave" => "MCU power save",
        "egpu_enable" => "eGPU",
        _ => "Setting",
    }
}

/// A plain-words line under a power-limit slider.
fn power_limit_hint(name: &str) -> &'static str {
    match name {
        "ppt_apu_sppt" => "Watts the processor (with its built-in graphics) can keep using under steady load",
        "ppt_platform_sppt" => "Total watts shared by the processor and the graphics card",
        "ppt_pl1_spl" => "Watts the processor can keep using for as long as the load lasts",
        "ppt_pl2_sppt" => "Extra watts the processor may use for a few minutes at a time",
        "ppt_fppt" | "ppt_pl3_fppt" => "Extra watts the processor may use for a few seconds at a time",
        "nv_dynamic_boost" => "Watts the graphics card may borrow from the processor when it's busy",
        "nv_temp_target" => "How hot the graphics card may run before it slows down",
        "nv_tgp" => "Watts the graphics card may use",
        _ => "",
    }
}

fn power_limit_unit(name: &str) -> &'static str {
    if name == "nv_temp_target" { "°C" } else { "W" }
}

struct Shot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    done: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

pub struct CrateApp {
    page: Page,
    /// Hardware readings come from a background thread, so a slow firmware read
    /// (for example while the discrete GPU powers up) never freezes the window.
    snap_rx: Receiver<Snapshot>,
    snap_kick: Sender<()>,
    snap: Snapshot,
    last_sample: Instant,
    hist_cpu_temp: VecDeque<f32>,
    hist_cpu_load: VecDeque<f32>,
    hist_gpu_temp: VecDeque<f32>,
    hist_gpu_load: VecDeque<f32>,
    /// Load of each GPU on its own (the Home load monitors); 0 while a GPU sleeps.
    hist_dgpu_load: VecDeque<f32>,
    hist_igpu_load: VecDeque<f32>,
    /// Battery charge or drain (W) over the last minute.
    hist_batt_w: VecDeque<f32>,
    /// Height of the visible page area (for the 3×3 Home grid).
    view_h: f32,
    /// Whether the window has been fitted to a small screen yet (first frames only).
    screen_fitted: bool,
    /// System RAM and GPU memory in use, in percent.
    hist_mem: VecDeque<f32>,
    hist_vram: VecDeque<f32>,
    hist_fans: Vec<VecDeque<f32>>,
    tools: Tools,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: usize,
    log: Vec<(Instant, Outcome)>,
    toast: Option<(Instant, Outcome)>,
    settings: Settings,
    /// A dragged fan curve waiting to be sent: (mode, when it last changed).
    fan_pending: Option<(String, Instant)>,
    /// The mode whose stock fan curve is being read.
    stock_fetching: Option<String>,
    charge_edit: Option<u8>,
    attr_edit: BTreeMap<String, i64>,
    /// Slider values just sent: shown until the readings catch up (or a few seconds pass).
    limits_sent: Option<(Instant, BTreeMap<String, i64>)>,
    reboot_pending: bool,
    shot: Option<Shot>,
    /// "show" / "quit" requests from the tray process (or a second launch).
    tray_rx: Receiver<TrayCmd>,
    tray_error: Option<String>,
    /// Closing for good: also stop the tray process.
    quitting: bool,
    /// Charger plugged in / out, for "60 Hz on battery".
    power: crate::display::PowerWatcher,
    /// The screen's current mode, e.g. "2560×1600 at 120 Hz" (read in the background).
    panel_info: Arc<Mutex<Option<String>>>,
    /// Curves read back after a fan reset: (mode, CPU and GPU curves if asusctl answered).
    fan_reload: Arc<Mutex<Option<(String, Option<(hw::Curve, hw::Curve)>)>>>,
    /// The desktop's light/dark preference (None = unknown), kept current in the background.
    system_dark: Arc<Mutex<Option<bool>>>,
    /// `--theme` on the command line (for checks); otherwise the setting decides.
    theme_override: Option<settings::Theme>,
    autostart: bool,
    started: Instant,
    ctx: egui::Context,
    distro: setup::Distro,
    last_tool_check: Instant,
    /// A lighting effect change waiting to be sent (sent shortly after the last edit).
    aura_pending: Option<Instant>,
    aura_running: bool,
    /// A graphics mode that needs a restart, waiting for the person to confirm.
    gfx_confirm: Option<GfxMode>,
    /// Restart as soon as the confirmed graphics change has been written.
    restart_after_gfx: bool,
    restart_confirm: bool,
    /// "Shut down now?" (from the stuck-fans warning).
    shutdown_confirm: bool,
    fans_stuck: crate::hw::StuckFans,
    fans_stuck_now: bool,
}

impl CrateApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        screenshot: Option<PathBuf>,
        page: Option<String>,
        gfx: Option<GfxMode>,
        theme_override: Option<settings::Theme>,
    ) -> Self {
        // Colours: the chosen theme, or the desktop's light/dark preference (looked up now
        // so the window opens in the right colours, then every few seconds in the background).
        let system_dark: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));
        {
            let theme = theme_override.unwrap_or(Settings::load().theme);
            if theme == settings::Theme::System {
                *system_dark.lock().unwrap() = crate::widgets::system_prefers_dark();
            }
            let slot = system_dark.clone();
            let c = cc.egui_ctx.clone();
            std::thread::Builder::new()
                .name("system-theme".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(4));
                    let now = crate::widgets::system_prefers_dark();
                    let changed = slot.lock().map(|mut g| std::mem::replace(&mut *g, now) != now).unwrap_or(false);
                    if changed {
                        c.request_repaint();
                    }
                })
                .ok();
            let system = system_dark.lock().ok().and_then(|g| *g);
            crate::widgets::set_palette(palette_for(theme, system));
            crate::widgets::set_calm(Settings::load().neon_calm);
        }
        apply_theme(&cc.egui_ctx);
        let (tx, rx) = channel();
        let (tray_tx, tray_rx) = channel();
        if screenshot.is_none() {
            // Requests from the tray process and from a second launch.
            let c = cc.egui_ctx.clone();
            let _ = crate::daemon::listen(&crate::daemon::window_socket(), move |line| {
                let cmd = match line.as_str() {
                    "show" => TrayCmd::Show(None),
                    "quit" => TrayCmd::Quit,
                    other => match other.strip_prefix("gfx ").and_then(GfxMode::from_key) {
                        Some(m) => TrayCmd::Gfx(m, None),
                        None => return,
                    },
                };
                let _ = tray_tx.send(cmd);
                c.request_repaint();
            });
        }
        let mut sampler = Sampler::default();
        let snap = sampler.sample();
        let (snap_tx, snap_rx) = channel::<Snapshot>();
        let (snap_kick, kick_rx) = channel::<()>();
        {
            let c = cc.egui_ctx.clone();
            std::thread::Builder::new()
                .name("hw-sampler".into())
                .spawn(move || loop {
                    // Read once a second, or right away when asked (after a change).
                    match kick_rx.recv_timeout(Duration::from_millis(1000)) {
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                        // After a change: read everything fresh, Cardwire's mode included.
                        Ok(()) => {
                            while kick_rx.try_recv().is_ok() {}
                            sampler.forget_cardwire();
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    if snap_tx.send(sampler.sample()).is_err() {
                        break;
                    }
                    c.request_repaint();
                })
                .expect("start the hardware sampler");
        }
        let settings = Settings::load();
        let ctx = cc.egui_ctx.clone();
        let c2 = ctx.clone();
        ctl::detect_tools(tx.clone(), move || c2.request_repaint());

        let mut app = Self {
            page: Page::Home,
            snap_rx,
            snap_kick,
            snap,
            last_sample: Instant::now(),
            hist_cpu_temp: VecDeque::new(),
            hist_cpu_load: VecDeque::new(),
            hist_gpu_temp: VecDeque::new(),
            hist_gpu_load: VecDeque::new(),
            hist_dgpu_load: VecDeque::new(),
            hist_igpu_load: VecDeque::new(),
            hist_batt_w: VecDeque::new(),
            view_h: 800.0,
            screen_fitted: false,
            hist_mem: VecDeque::new(),
            hist_vram: VecDeque::new(),
            hist_fans: Vec::new(),
            tools: Tools::default(),
            tx,
            rx,
            busy: 0,
            log: Vec::new(),
            // ARCHERYCRATE_SHOW_TOAST=1: a screenshot with a "just applied" message in the status bar.
            toast: (screenshot.is_some() && std::env::var_os("ARCHERYCRATE_SHOW_TOAST").is_some()).then(|| {
                (
                    Instant::now(),
                    Outcome { label: "Custom fan curves on for Turbo".into(), ok: true, via: "asusctl".into(), log: vec![], needs_reboot: false },
                )
            }),
            settings,
            fan_pending: None,
            stock_fetching: None,
            charge_edit: None,
            attr_edit: BTreeMap::new(),
            limits_sent: None,
            reboot_pending: false,
            shot: screenshot.map(|path| Shot {
                path,
                frames: 0,
                requested: false,
                done: Arc::new(AtomicBool::new(false)),
                error: Arc::new(Mutex::new(None)),
            }),
            tray_rx,
            tray_error: None,
            quitting: false,
            power: Default::default(),
            panel_info: Default::default(),
            fan_reload: Default::default(),
            system_dark,
            theme_override,
            autostart: tray::autostart_enabled(),
            started: Instant::now(),
            ctx,
            distro: setup::detect(),
            last_tool_check: Instant::now(),
            aura_pending: None,
            aura_running: false,
            gfx_confirm: None,
            restart_after_gfx: false,
            restart_confirm: false,
            shutdown_confirm: false,
            fans_stuck: Default::default(),
            fans_stuck_now: false,
        };
        if let Some(p) = page {
            if let Some(pg) = Page::ALL.iter().find(|x| x.title().eq_ignore_ascii_case(&p)) {
                app.page = *pg;
            }
        }
        if let Some(m) = gfx {
            if app.shot.is_some() {
                // A picture of the question only; never switch from a screenshot run.
                app.page = Page::Graphics;
                app.gfx_confirm = Some(m);
            } else {
                // Opened from the tray's graphics menu.
                app.pick_gfx(m);
            }
        }
        app.seed_curves_from_firmware();
        app.push_history();
        if app.shot.is_none() && app.settings.tray_icon {
            app.start_tray();
        }
        app.refresh_panel_info(Duration::ZERO);
        app
    }

    /// Switch the palette when the theme setting or the desktop's preference changes.
    fn follow_theme(&mut self, ctx: &egui::Context) {
        let system = self.system_dark.lock().ok().and_then(|g| *g);
        let theme = self.theme_override.unwrap_or(self.settings.theme);
        if crate::widgets::set_palette(palette_for(theme, system)) {
            apply_theme(ctx);
        }
        // Calm: always (Settings), or only while on battery (Battery page).
        let on_battery = self.snap.on_ac == Some(false);
        crate::widgets::set_calm(self.settings.neon_calm || (self.settings.battery_calm && on_battery));
        crate::widgets::set_rainbow_speed(self.settings.rainbow_speed.unwrap_or(1.0));
        // ARCHERYCRATE_BOLD_GRAPHS=1 turns them on for a screenshot without changing the setting.
        let bold_shot = self.shot.is_some() && std::env::var_os("ARCHERYCRATE_BOLD_GRAPHS").is_some();
        let neon_theme = matches!(theme, settings::Theme::Neon | settings::Theme::NeonLight);
        let bold = if neon_theme { !self.settings.neon_plain_graphs } else { self.settings.bold_graphs };
        // ARCHERYCRATE_PLAIN_GRAPHS=1 turns them off for a screenshot.
        let plain_shot = self.shot.is_some() && std::env::var_os("ARCHERYCRATE_PLAIN_GRAPHS").is_some();
        crate::widgets::set_bold_graphs((bold || bold_shot) && !plain_shot);
        // Neon's rainbow moves: keep drawing (about 30 times a second) while it does.
        if crate::widgets::rainbow_moving() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    /// A stock-curve reading from `fetch_stock` arrived: remember it for that mode.
    fn take_stock(&mut self) {
        let Some((mode, curves)) = self.fan_reload.lock().ok().and_then(|mut g| g.take()) else { return };
        self.stock_fetching = None;
        if let Some((cpu, gpu)) = curves {
            self.settings.stock.insert(mode.clone(), settings::StockCurves { cpu, gpu });
            let pc = self.settings.curves_for(&mode);
            if !pc.enabled {
                pc.cpu = cpu;
                pc.gpu = gpu;
            }
            self.settings.save();
        }
        let _ = self.snap_kick.send(());
    }

    /// Read the screen's mode in the background (after `delay`, to let a change settle).
    fn refresh_panel_info(&self, delay: Duration) {
        let info = self.panel_info.clone();
        let c = self.ctx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let d = crate::display::panel().map(|p| {
                let max = p.max_hz();
                format!("{} (up to {max:.0} Hz)", p.describe())
            });
            if let Ok(mut g) = info.lock() {
                *g = d;
            }
            c.request_repaint();
        });
    }


    /// The tray icon runs in its own process (see `daemon`), so it survives the window closing.
    fn start_tray(&mut self) {
        self.tray_error = crate::daemon::ensure_tray().err();
    }

    /// Close the window but keep ArcheryCrate running in the tray.
    fn close_to_tray(&mut self) {
        self.start_tray();
        self.ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// Close the window and the tray icon.
    fn quit(&mut self) {
        self.quitting = true;
        self.ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    /// The firmware holds the curve of the active profile; start the editor from it.
    fn seed_curves_from_firmware(&mut self) {
        if let (Some(p), Some(cpu), Some(gpu)) = (self.snap.profile.clone(), self.snap.cpu_curve, self.snap.gpu_curve) {
            if !self.settings.curves.contains_key(&p) {
                let enabled = self.snap.cpu_curve_enabled.unwrap_or(false);
                let pc = self.settings.curves_for(&p);
                pc.cpu = cpu;
                pc.gpu = gpu;
                pc.enabled = enabled;
            }
        }
    }

    fn push_history(&mut self) {
        // About one reading a second: 15 minutes, the longest range Home can show.
        let cap = 900;
        let push = |q: &mut VecDeque<f32>, v: f32| {
            q.push_back(v);
            if q.len() > cap {
                q.pop_front();
            }
        };
        if let Some(t) = self.snap.cpu_temp_c {
            push(&mut self.hist_cpu_temp, t);
        }
        if let Some(l) = self.snap.cpu_load_pct {
            push(&mut self.hist_cpu_load, l);
        }
        if let Some(d) = self.snap.dgpu() {
            push(&mut self.hist_dgpu_load, if d.awake { d.busy_pct.unwrap_or(0) as f32 } else { 0.0 });
        }
        if let Some(i) = self.snap.igpu() {
            push(&mut self.hist_igpu_load, i.busy_pct.unwrap_or(0) as f32);
        }
        if let Some(g) = home_gpu(&self.snap) {
            push(&mut self.hist_gpu_temp, g.temp_c.unwrap_or(0.0));
            push(&mut self.hist_gpu_load, g.busy_pct.unwrap_or(0) as f32);
            if let (Some(u), Some(t)) = (g.vram_used_mb, g.vram_total_mb) {
                push(&mut self.hist_vram, 100.0 * u as f32 / t.max(1) as f32);
            }
        }
        if let (Some(u), Some(t)) = (self.snap.mem_used_gb, self.snap.mem_total_gb) {
            push(&mut self.hist_mem, 100.0 * u / t.max(0.1));
        }
        while self.hist_fans.len() < self.snap.fans.len() {
            self.hist_fans.push(VecDeque::new());
        }
        for (i, (_, rpm)) in self.snap.fans.iter().enumerate() {
            push(&mut self.hist_fans[i], *rpm as f32);
        }
        // Battery draw while charging or draining, for a steady "time left".
        match (self.snap.battery_power_w, self.snap.battery_status.as_str()) {
            (Some(w), "Discharging" | "Charging") if w > 0.1 => {
                self.hist_batt_w.push_back(w);
                if self.hist_batt_w.len() > 60 {
                    self.hist_batt_w.pop_front();
                }
            }
            _ => self.hist_batt_w.clear(),
        }
    }

    fn run(&mut self, action: Action) {
        self.busy += 1;
        let c = self.ctx.clone();
        ctl::execute(action, self.tools.clone(), self.tx.clone(), move || c.request_repaint());
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Tools(t) => {
                    self.tools = t;
                    self.last_tool_check = Instant::now();
                }
                Msg::Done(o) => {
                    self.busy = self.busy.saturating_sub(1);
                    if o.label.starts_with(ctl::AURA_LABEL) {
                        self.aura_running = false;
                    }
                    if o.label.contains("asusctl") || o.label.contains("Cardwire") {
                        self.recheck_tools();
                    }
                    if o.ok && o.label.starts_with(ctl::GFX_LABEL) {
                        // Taking a waiting switch back clears it again.
                        self.reboot_pending = o.needs_reboot;
                    } else if o.ok && o.needs_reboot {
                        self.reboot_pending = true;
                    }
                    let restart_now = o.label.starts_with(ctl::GFX_LABEL) && std::mem::take(&mut self.restart_after_gfx);
                    if restart_now && o.ok {
                        self.run(ctl::restart_laptop());
                    }
                    self.toast = Some((Instant::now(), o.clone()));
                    self.log.push((Instant::now(), o));
                    let _ = self.snap_kick.send(());
                }
            }
        }
        while let Ok(cmd) = self.tray_rx.try_recv() {
            match cmd {
                TrayCmd::Show(_) => {
                    self.ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    self.ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayCmd::Profile(p) => self.run(ctl::set_profile(&p)),
                TrayCmd::Gfx(m, _) => {
                    self.ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    self.ctx.send_viewport_cmd(ViewportCommand::Focus);
                    self.pick_gfx(m);
                }
                // The tray process is already quitting; just close.
                // Quit from the tray menu: close for good, don't start the tray again.
                TrayCmd::Quit => {
                    self.quitting = true;
                    self.ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        let mut fresh = None;
        while let Ok(s) = self.snap_rx.try_recv() {
            fresh = Some(s);
        }
        if let Some(s) = fresh {
            self.snap = s;
            if self.shot.is_none() && self.tools.checked {
                if let Some(on_battery) = self.power.changed(self.snap.on_ac) {
                    // The tray process handles the charger when it runs; otherwise do it here.
                    if !self.settings.tray_icon {
                        let c = self.ctx.clone();
                        crate::power::on_change(on_battery, &self.tools, self.tx.clone(), move || c.request_repaint());
                    }
                    self.refresh_panel_info(Duration::from_millis(2500));
                }
            }
            self.fans_stuck_now = self.fans_stuck.update(&self.snap);
            // Extra readings right after a change shouldn't squeeze the history graphs.
            if self.last_sample.elapsed() >= Duration::from_millis(900) {
                self.last_sample = Instant::now();
                self.push_history();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover());
            let p = ui.painter();
            if crate::widgets::is_neon() {
                // The ring logo cycles through the rainbow, with a glow.
                let c = crate::widgets::rainbow(0.0);
                p.circle_stroke(r.center(), 14.5, egui::Stroke::new(4.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 50)));
                p.circle_stroke(r.center(), 12.0, egui::Stroke::new(3.5, c));
            } else {
                p.circle_stroke(r.center(), 12.0, egui::Stroke::new(3.5, ACCENT()));
            }
            p.circle_filled(r.center(), 5.0, TEXT());
            ui.vertical(|ui| {
                ui.label(RichText::new("ArcheryCrate").size(18.0).strong().color(TEXT()));
                ui.label(RichText::new("control center").size(11.0).color(DIM()));
            });
        });
        ui.add_space(18.0);
        for page in Page::ALL {
            let sel = self.page == page;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
            let p = ui.painter();
            if sel {
                crate::widgets::glow_rect(p, rect.shrink2(vec2(8.0, 2.0)), 8.0, ACCENT());
                p.rect_filled(rect.shrink2(vec2(8.0, 2.0)), 8.0, CARD_HI());
                p.rect_filled(
                    egui::Rect::from_min_size(rect.left_top() + vec2(8.0, 10.0), vec2(3.0, 20.0)),
                    2.0,
                    ACCENT(),
                );
            } else if resp.hovered() {
                crate::widgets::glow_rect(p, rect.shrink2(vec2(8.0, 2.0)), 8.0, CYAN());
                p.rect_filled(rect.shrink2(vec2(8.0, 2.0)), 8.0, CARD());
            }
            let col = if sel { TEXT() } else { DIM() };
            let icon_c = rect.left_center() + vec2(33.0, 0.0);
            if crate::widgets::is_neon() {
                // Neon: glowing line icons, each in its own neon colour (dimmer when not chosen).
                let col = crate::widgets::neon_icon_color(page.key());
                let col = if sel || resp.hovered() { col } else { lerp_color(col, SIDEBAR(), 0.35) };
                crate::widgets::neon_icon(p, page.key(), icon_c, 10.0, col);
            } else {
                // Dark and Light: the same line icons, flat, in the theme's colours.
                let col = if sel { ACCENT() } else if resp.hovered() { TEXT() } else { DIM() };
                crate::widgets::neon_icon(p, page.key(), icon_c, 10.0, col);
            }
            p.text(
                rect.left_center() + vec2(54.0, 0.0),
                egui::Align2::LEFT_CENTER,
                page.title(),
                egui::FontId::proportional(15.0),
                col,
            );
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.page = page;
            }
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            let s = &self.snap;
            if let Some(t) = s.cpu_temp_c {
                pill(ui, &format!("CPU {t:.0}°C"), temp_color(t));
            }
            // The GPU Home follows: the discrete one while awake, else the integrated one.
            let gpu = home_gpu(s);
            if let Some(t) = gpu.as_ref().and_then(|g| g.temp_c) {
                pill(ui, &format!("GPU {t:.0}°C"), temp_color(t));
            }
            for (label, rpm) in &s.fans {
                pill(ui, &format!("{} {rpm} rpm", fan_label(label)), DIM());
            }
            if let Some(b) = s.battery_pct {
                let plug = if s.on_ac == Some(true) { " ⚡" } else { "" };
                pill(ui, &format!("Battery {b}%{plug}"), battery_color(b as f32));
            }
            if let (Some(u), Some(t)) = (s.mem_used_gb, s.mem_total_gb) {
                let f = u / t.max(0.1);
                pill(ui, &format!("RAM {:.0}%", f * 100.0), level_color(f, DIM()));
            }
            if let Some((Some(u), Some(t))) = gpu.as_ref().map(|g| (g.vram_used_mb, g.vram_total_mb)) {
                let f = u as f32 / t.max(1) as f32;
                pill(ui, &format!("GPU RAM {:.0}%", f * 100.0), level_color(f, DIM()));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                // Short, so it never runs under the chips; the details are on the Activity page.
                // When even that doesn't fit, it's left out.
                let room = ui.available_width() > 190.0;
                if self.busy > 0 && room {
                    ui.spinner();
                    ui.label(RichText::new("Applying…").color(DIM()));
                } else if let (Some((at, o)), true) = (&self.toast, room) {
                    if at.elapsed() < Duration::from_secs(8) {
                        let (c, text) = if o.ok { (GREEN(), "✔ Settings applied") } else { (RED(), "✖ Not applied, see Activity") };
                        ui.label(RichText::new(text).color(c)).on_hover_text(&o.label);
                    }
                }
            });
        });
    }

    fn page_ui(&mut self, ui: &mut Ui) {
        if self.reboot_pending || self.snap.pending_reboot || self.snap.gfx_pending.is_some() {
            egui::Frame::new()
                .fill(lerp_color(CARD(), AMBER(), 0.15))
                .stroke(egui::Stroke::new(1.0, AMBER()))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let row = vec2(ui.available_width(), 36.0);
                    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("⟳  A graphics change is waiting for a restart of the laptop.").color(TEXT()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if accent_button(ui, "Restart Now").clicked() {
                                self.restart_confirm = true;
                            }
                        });
                    });
                });
            ui.add_space(8.0);
        }
        if self.fans_stuck_now && self.page != Page::Activity {
            egui::Frame::new()
                .fill(lerp_color(CARD(), RED(), 0.10))
                .stroke(egui::Stroke::new(1.0, lerp_color(LINE(), RED(), 0.6)))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("⚠  Fans stuck at full speed").color(TEXT()).strong());
                    ui.label(RichText::new(crate::hw::STUCK_FANS_TEXT).color(DIM()));
                    ui.add_space(4.0);
                    if accent_button(ui, "Shut down now…").clicked() {
                        self.shutdown_confirm = true;
                    }
                });
            ui.add_space(8.0);
        }
        let missing = self.tools.checked && self.tools.asusctl.is_none();
        let stopped = self.tools.checked && self.tools.asusctl.is_some() && !self.tools.asusd_active;
        if (missing || stopped) && self.page != Page::System && self.page != Page::Activity {
            egui::Frame::new()
                .fill(lerp_color(CARD(), AMBER(), 0.10))
                .stroke(egui::Stroke::new(1.0, lerp_color(LINE(), AMBER(), 0.6)))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let msg = if missing {
                            "asusctl isn't installed. Keyboard lighting needs it, and other changes will ask for your password each time."
                        } else {
                            "asusctl is installed but its service isn't running."
                        };
                        ui.label(RichText::new(format!("⚠  {msg}")).color(TEXT()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if accent_button(ui, if missing { "Set up asusctl" } else { "Fix it" }).clicked() {
                                self.page = Page::System;
                            }
                        });
                    });
                });
            ui.add_space(8.0);
        }
        match self.page {
            Page::Home => self.home(ui),
            Page::Summary => self.summary(ui),
            Page::Performance => self.performance(ui),
            Page::Fans => self.fans(ui),
            Page::Battery => self.battery(ui),
            Page::Lighting => self.lighting(ui),
            Page::Graphics => self.graphics(ui),
            Page::System => self.system(ui),
            Page::Activity => self.activity(ui),
        }
    }

    // -----------------------------------------------------------------------
    // Pages
    // -----------------------------------------------------------------------

    fn profile_tiles(&mut self, ui: &mut Ui, compact: bool) {
        let mut choices = self.snap.profile_choices.clone();
        if choices.is_empty() {
            choices = vec!["quiet".into(), "balanced".into(), "performance".into()];
        }
        choices.retain(|c| c != "custom");
        let current = self.snap.profile.clone().unwrap_or_default();
        let n = choices.len().max(1) as f32;
        let w = ((ui.available_width() - 12.0 * (n - 1.0)) / n).max(140.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for c in &choices {
                let (icon, desc) = match c.as_str() {
                    "quiet" | "low-power" => ("🌙", "Lowest noise and power. Fans stay calm; best for battery and quiet rooms."),
                    "performance" => ("🚀", "Highest power limits and aggressive cooling for games and heavy work."),
                    _ => ("⚖", "Balanced power and noise for everyday use."),
                };
                let desc = if compact { "" } else { desc };
                if mode_tile(ui, w, icon, profile_title(c), desc, *c == current, profile_color(c)).clicked() && *c != current {
                    self.run(ctl::set_profile(c));
                }
            }
        });
    }

    /// Everything ArcheryCrate manages, as it stands now, in one list.
    fn summary(&mut self, ui: &mut Ui) {
        page_title(ui, "Summary", "The current state of everything ArcheryCrate manages.");
        let s = self.snap.clone();
        let st = self.settings.clone();
        let t = self.tools.clone();
        let on_off = |b: bool| if b { "on".to_string() } else { "off".to_string() };
        let attr_on = |n: &str| s.attr(n).and_then(|a| a.current).map(|v| on_off(v != 0)).unwrap_or("–".into());

        // Fan curves per mode: yours or the laptop's own.
        let mut modes = s.profile_choices.clone();
        modes.retain(|m| m != "custom");
        if modes.is_empty() {
            modes = vec!["quiet".into(), "balanced".into(), "performance".into()];
        }
        let fan_rows: Vec<(String, String)> = modes
            .iter()
            .map(|m| {
                let mine = st.curves.get(m).is_some_and(|c| c.enabled);
                (format!("{} fans", profile_title(m)), if mine { "custom curve".into() } else { "stock curve".into() })
            })
            .collect();

        // "On battery" options that are switched on.
        let mut auto: Vec<String> = Vec::new();
        if st.battery_silent {
            auto.push("Silent mode".into());
        }
        if st.battery_silent_fans {
            auto.push("silent fans".into());
        }
        if st.battery_eco {
            auto.push("Eco graphics".into());
        }
        if st.battery_60hz {
            auto.push("60 Hz".into());
        }
        if st.battery_overdrive_off {
            auto.push("panel overdrive off".into());
        }
        if st.battery_backlight {
            auto.push(format!("screen {}%", st.battery_backlight_pct));
        }
        if st.kbd_off_on_battery {
            auto.push("keyboard light off".into());
        } else if st.battery_kbd_dim {
            auto.push(format!("keyboard light {}", ["low", "medium", "high"][(st.battery_kbd_level.clamp(1, 3) - 1) as usize]));
        }

        let a = &st.aura;
        let mut lit: Vec<&str> = Vec::new();
        for (on, w) in [(a.awake, "in use"), (a.boot, "starting"), (a.sleep, "asleep"), (a.shutdown, "shutting down")] {
            if on {
                lit.push(w);
            }
        }
        let effect = if a.mode.uses_color() {
            format!("{} #{:02X}{:02X}{:02X}", a.mode.title(), a.color1[0], a.color1[1], a.color1[2])
        } else {
            a.mode.title().to_string()
        };
        let panel = self.panel_info.lock().ok().and_then(|g| g.clone()).unwrap_or("–".into());
        let ppts: Vec<(String, String)> = s
            .attrs
            .iter()
            .filter(|x| x.name.starts_with("ppt_") || x.name.starts_with("nv_"))
            .map(|x| (attr_title(&x.name).to_string(), x.current.map(|v| format!("{v} W")).unwrap_or("–".into())))
            .collect();

        ui.columns(2, |cols| {
            card(&mut cols[0], "Performance and fans", |ui| {
                stat_row(ui, "Performance mode", profile_title(s.profile.as_deref().unwrap_or("")).to_string());
                for (k, v) in &ppts {
                    stat_row(ui, k, v.clone());
                }
                for (k, v) in &fan_rows {
                    stat_row(ui, k, v.clone());
                }
            });
            card(&mut cols[0], "Battery", |ui| {
                stat_row(ui, "Charge limit", s.charge_limit.map(|c| format!("{c}%")).unwrap_or("–".into()));
                stat_row(ui, "Charger", match s.on_ac {
                    Some(true) => "plugged in".into(),
                    Some(false) => "unplugged".into(),
                    None => "–".into(),
                });
                if auto.is_empty() {
                    stat_row(ui, "On battery", "nothing changes".into());
                } else {
                    ui.label(RichText::new("On battery").color(DIM()));
                    ui.add(egui::Label::new(RichText::new(auto.join(" · ")).color(TEXT())).wrap());
                }
            });
            card(&mut cols[1], "Graphics and display", |ui| {
                stat_row(ui, "Graphics mode", s.gfx_mode().title().to_string());
                stat_row(ui, "Eco switching", match (&s.cardwire_mode, t.cardwire) {
                    (Some(_), _) | (_, Some(true)) => "Cardwire".into(),
                    (_, Some(false)) => "Cardwire (service stopped)".into(),
                    _ => "firmware switch".into(),
                });
                stat_row(ui, "Screen", panel.clone());
                stat_row(ui, "Panel overdrive", attr_on("panel_overdrive"));
                stat_row(ui, "Start-up sound", attr_on("boot_sound"));
                if s.pending_reboot || self.reboot_pending {
                    stat_row(ui, "Restart", "needed to finish a change".into());
                }
            });
            card(&mut cols[1], "Keyboard", |ui| {
                stat_row(ui, "Effect", effect.clone());
                if a.mode.uses_speed() {
                    stat_row(ui, "Speed", ["slow", "medium", "fast"][a.speed.min(2) as usize].into());
                }
                stat_row(ui, "Brightness", s.kbd_brightness.map(|b| ctl::kbd_level_name(b).to_string()).unwrap_or("–".into()));
                stat_row(ui, "Lit while", if lit.is_empty() { "never".into() } else { lit.join(", ") });
            });
            card(&mut cols[1], "App", |ui| {
                stat_row(ui, "Tray icon", on_off(st.tray_icon));
                stat_row(ui, "Closing the window", if st.close_to_tray { "keeps it in the tray".into() } else { "quits".into() });
                stat_row(ui, "Start at login", on_off(self.autostart));
                stat_row(ui, "asusctl", t.asusctl.clone().unwrap_or("not installed".into()));
            });
        });
    }

    fn home(&mut self, ui: &mut Ui) {
        // The laptop itself is the heading here.
        page_title(ui, &format!("{} · {}", short_model(&self.snap.model), self.snap.cpu_name), "");
        // ARCHERYCRATE_HOME_GRID=1 shows the grid in a screenshot run without changing the setting.
        let grid_shot = self.shot.is_some() && std::env::var_os("ARCHERYCRATE_HOME_GRID").is_some();
        // ARCHERYCRATE_HOME_DETAILED=1 does the opposite.
        let detailed_shot = self.shot.is_some() && std::env::var_os("ARCHERYCRATE_HOME_DETAILED").is_some();
        if (self.settings.home_grid || grid_shot) && !detailed_shot {
            self.home_grid(ui);
            return;
        }
        let s = self.snap.clone();
        let g = ((ui.available_width() - 36.0) / 4.0 - 40.0).clamp(100.0, 150.0);
        let secs = self.settings.home_range_secs.unwrap_or(60) as usize;
        let gpu = home_gpu(&s);
        let gpu_name = match &gpu {
            Some(x) if x.discrete => "Discrete GPU",
            Some(_) => "Integrated GPU",
            None => "GPU",
        };
        let package_w = s.igpu().and_then(|x| x.power_w);
        let batt_avg = (!self.hist_batt_w.is_empty())
            .then(|| self.hist_batt_w.iter().sum::<f32>() / self.hist_batt_w.len() as f32);

        // 3 + 4. One load graph for the CPU and both GPUs; an idle discrete GPU is dimmed.
        let series_color = |k: usize, fixed: Color32| if bold_graphs() { rainbow(k as f32 / 3.0) } else { fixed };
        let clock_text = |m: u32| if m == 0 { "clock idle".to_string() } else { format!("{m} MHz") };
        let mut cols = vec![Col {
            name: "CPU".into(),
            color: series_color(0, CYAN()),
            value: s.cpu_load_pct.map(|l| format!("{l:.0}%")),
            note: String::new(),
            lines: vec![
                s.cpu_mhz.map(|m| format!("{:.2} GHz average", m as f32 / 1000.0)).unwrap_or_default(),
                package_w.map(|w| format!("{w:.1} W package (CPU + integrated graphics)")).unwrap_or_default(),
            ],
            dim: false,
            hist: recent(&self.hist_cpu_load, secs),
        }];
        let eco = s.gfx_mode() == GfxMode::Eco;
        match s.dgpu() {
            Some(d) if d.awake => cols.push(Col {
                name: "Discrete GPU".into(),
                color: series_color(1, ACCENT()),
                value: d.busy_pct.map(|b| format!("{b}%")),
                note: String::new(),
                lines: vec![
                    d.clock_mhz.map(clock_text).unwrap_or_default(),
                    d.power_w.map(|w| format!("{w:.1} W")).unwrap_or_default(),
                ],
                dim: false,
                hist: recent(&self.hist_dgpu_load, secs),
            }),
            other => cols.push(Col {
                name: "Discrete GPU".into(),
                color: series_color(1, ACCENT()),
                value: None,
                note: if eco || other.is_none() { "Off (Eco)".into() } else { "Asleep".into() },
                lines: vec![
                    if eco || other.is_none() { "switched off to save power".into() } else { "saving power until a game or app needs it".into() },
                    String::new(),
                ],
                dim: true,
                hist: recent(&self.hist_dgpu_load, secs),
            }),
        }
        if let Some(i) = s.igpu() {
            cols.push(Col {
                name: "Integrated GPU".into(),
                color: series_color(2, GREEN()),
                value: i.busy_pct.map(|b| format!("{b}%")),
                note: String::new(),
                lines: vec![
                    i.clock_mhz.map(clock_text).unwrap_or_default(),
                    "power: shared with the CPU".into(),
                ],
                dim: false,
                hist: recent(&self.hist_igpu_load, secs),
            });
        }
        card(ui, "Load", |ui| {
            combined_graph(ui, &cols, (0.0, 100.0), &[25.0, 50.0, 75.0, 100.0], "%", 150.0);
            let peak = |q: &VecDeque<f32>| recent_peak(q, secs).map(|p| format!("{p:.0}%")).unwrap_or("–".into());
            ui.label(
                RichText::new(format!(
                    "{} · peak: CPU {} · discrete GPU {} · integrated GPU {}",
                    span_text(self.hist_cpu_load.len(), secs),
                    peak(&self.hist_cpu_load),
                    peak(&self.hist_dgpu_load),
                    peak(&self.hist_igpu_load),
                ))
                .color(DIM())
                .size(12.0),
            );
        });
        ui.add_space(4.0);

        // Temperatures, each with its fan's speed.
        let fan_rpm = |name: &str| s.fans.iter().find(|(l, _)| l == name).map(|(_, r)| *r);
        let temp_box = |ui: &mut Ui, now: Option<f32>, q: &VecDeque<f32>, fan: Option<u32>| {
            ui.horizontal(|ui| {
                match now {
                    Some(t) => {
                        ui.label(RichText::new(format!("{t:.0}")).size(40.0).strong().color(temp_color(t)));
                        ui.label(RichText::new("°C").size(18.0).color(temp_color(t)));
                    }
                    None => {
                        ui.label(RichText::new("–").size(40.0).color(DIM()));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.vertical(|ui| {
                        if let Some(rpm) = fan {
                            ui.label(RichText::new(format!("fan {rpm} rpm")).strong().color(CYAN()));
                        }
                        if let Some(p) = recent_peak(q, secs) {
                            ui.label(RichText::new(format!("peak {p:.0}°C")).color(DIM()));
                        }
                    });
                });
            });
            let color = now.map(temp_color).unwrap_or(DIM());
            temp_graph(ui, &recent(q, secs), color, 130.0);
            ui.label(RichText::new(span_text(q.len(), secs)).color(DIM()).size(12.0));
        };
        ui.columns(2, |c| {
            card(&mut c[0], "CPU temperature", |ui| {
                temp_box(ui, s.cpu_temp_c, &self.hist_cpu_temp, fan_rpm("cpu_fan"));
            });
            card(&mut c[1], &format!("{gpu_name} temperature"), |ui| {
                temp_box(ui, gpu.as_ref().and_then(|x| x.temp_c), &self.hist_gpu_temp, fan_rpm("gpu_fan"));
            });
        });
        ui.add_space(4.0);

        // Fans and memory, each as one combined graph like Load.
        let fan_cols: Vec<Col> = s
            .fans
            .iter()
            .enumerate()
            .map(|(i, (label, rpm))| {
                let q = self.hist_fans.get(i);
                Col {
                    name: fan_label(label),
                    color: series_color(i, if i == 0 { CYAN() } else { ACCENT() }),
                    value: Some(format!("{rpm} rpm")),
                    note: String::new(),
                    lines: vec![q
                        .and_then(|q| recent_peak(q, secs))
                        .map(|p| format!("peak {p:.0} rpm"))
                        .unwrap_or_default()],
                    dim: false,
                    hist: q.map(|q| recent(q, secs)).unwrap_or_default(),
                }
            })
            .collect();
        let pct = |u: f32, t: f32| 100.0 * u / t.max(0.001);
        let mut mem_cols = vec![];
        if let (Some(u), Some(t)) = (s.mem_used_gb, s.mem_total_gb) {
            let p = pct(u, t);
            mem_cols.push(Col {
                name: "System RAM".into(),
                color: series_color(0, GREEN()),
                value: Some(format!("{p:.0}%")),
                note: String::new(),
                lines: vec![format!("{u:.1} of {t:.1} GB")],
                dim: false,
                hist: recent(&self.hist_mem, secs),
            });
        }
        if let Some((Some(u), Some(t))) = gpu.as_ref().map(|x| (x.vram_used_mb, x.vram_total_mb)) {
            let p = pct(u as f32, t as f32);
            mem_cols.push(Col {
                name: format!("{gpu_name} memory"),
                color: series_color(1, CYAN()),
                value: Some(format!("{p:.0}%")),
                note: String::new(),
                lines: vec![format!("{:.1} of {:.1} GB", u as f32 / 1024.0, t as f32 / 1024.0)],
                dim: false,
                hist: recent(&self.hist_vram, secs),
            });
        }
        // Nearly full memory shows in amber, then red.
        for c in &mut mem_cols {
            let now = c.value.as_deref().and_then(|v| v.trim_end_matches('%').parse::<f32>().ok()).unwrap_or(0.0);
            if now >= 80.0 {
                c.color = level_color(now / 100.0, c.color);
            }
        }
        ui.columns(2, |c| {
            card(&mut c[0], "Fans", |ui| {
                if fan_cols.is_empty() {
                    ui.label(RichText::new("No fan sensors found").color(DIM()));
                } else {
                    combined_graph(ui, &fan_cols, (0.0, 6500.0), &[2000.0, 4000.0, 6000.0], "", 130.0);
                    ui.label(RichText::new(format!("{} · rpm", span_text(self.hist_fans.first().map_or(0, |q| q.len()), secs))).color(DIM()).size(12.0));
                }
            });
            card(&mut c[1], "Memory", |ui| {
                if mem_cols.is_empty() {
                    ui.label(RichText::new("Reading memory…").color(DIM()));
                } else {
                    combined_graph(ui, &mem_cols, (0.0, 100.0), &[25.0, 50.0, 75.0, 100.0], "%", 130.0);
                    ui.label(RichText::new(span_text(self.hist_mem.len(), secs)).color(DIM()).size(12.0));
                }
            });
        });
        ui.add_space(4.0);

        // Storage and battery.
        ui.columns(2, |cols| {
            card(&mut cols[0], "Storage", |ui| {
                if s.disks.is_empty() {
                    ui.label(RichText::new("Reading drives…").color(DIM()));
                }
                for d in &s.disks {
                    let used = d.used_frac();
                    let color = level_color(used, GREEN());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&d.name).strong().color(TEXT()));
                        ui.label(RichText::new(&d.mount).color(DIM()).size(12.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{} free of {}",
                                    crate::hw::human_bytes(d.avail),
                                    crate::hw::human_bytes(d.size)
                                ))
                                .color(TEXT()),
                            );
                        });
                    });
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, 5.0, CARD_HI());
                    let fill = egui::Rect::from_min_size(rect.min, vec2(rect.width() * used.clamp(0.0, 1.0), rect.height()));
                    p.rect_filled(fill, 5.0, color);
                    ui.add_space(4.0);
                }
            });
            // 5. Battery, with the time left (or to full) at the current draw.
            card(&mut cols[1], "Battery", |ui| {
                ui.horizontal(|ui| {
                    let b = s.battery_pct.unwrap_or(0) as f32;
                    let sub = match (batt_avg.or(s.battery_power_w), s.battery_status.as_str()) {
                        (Some(w), "Discharging") if w > 0.1 => format!("−{w:.1} W"),
                        (Some(w), "Charging") if w > 0.1 => format!("+{w:.1} W"),
                        (_, st) => st.to_string(),
                    };
                    gauge(ui, g.min(110.0), b / 100.0, &format!("{b:.0}"), "%", &sub, battery_color(b));
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        if let Some(eta) = battery_eta(&s, batt_avg) {
                            ui.label(RichText::new(eta).strong().color(TEXT()));
                        }
                        stat_row(ui, "Charger", match s.on_ac {
                            Some(true) => "plugged in".into(),
                            Some(false) => "unplugged".into(),
                            None => "–".into(),
                        });
                        stat_row(ui, "Charge limit", s.charge_limit.map(|c| format!("{c}%")).unwrap_or("–".into()));
                        stat_row(ui, "Health", s.battery_health_pct.map(|h| format!("{h:.0}% of new")).unwrap_or("–".into()));
                    });
                });
            });
        });
    }

    /// Home as a 3×3 grid of compact tiles that fits the window without scrolling:
    /// loads on top, then temperatures and memory, then fans, storage and battery.
    fn home_grid(&mut self, ui: &mut Ui) {
        let s = self.snap.clone();
        // The same time range as the detailed view (chosen there).
        let secs = self.settings.home_range_secs.unwrap_or(60) as usize;
        let hist = |q: &VecDeque<f32>| recent(q, secs);
        let batt_avg = (!self.hist_batt_w.is_empty())
            .then(|| self.hist_batt_w.iter().sum::<f32>() / self.hist_batt_w.len() as f32);
        let gap = 8.0;
        // What's left under the page title, split into three rows.
        let used = ui.min_rect().height();
        // Between rows: our gap plus egui's own item spacing.
        let between = gap + ui.spacing().item_spacing.y;
        let tile_h = ((self.view_h - used - 2.0 * between - 12.0) / 3.0).max(170.0);
        let package_w = s.igpu().and_then(|x| x.power_w);
        let clock_text = |m: u32| if m == 0 { "clock idle".to_string() } else { format!("{m} MHz") };
        let load_graph = |ui: &mut Ui, h: &[f32]| {
            let gh = (ui.available_height() - 4.0).max(36.0);
            history_graph(ui, h, (0.0, 100.0), &[50.0, 100.0], "%", "%", CYAN(), gh, Line::Rainbow);
        };
        let gpu = home_gpu(&s);
        let gpu_title = match &gpu {
            Some(x) if x.discrete => "Discrete GPU temperature",
            Some(_) => "Integrated GPU temperature",
            None => "GPU temperature",
        };

        // Row 1: loads.
        ui.columns(3, |c| {
            card_sized(&mut c[0], "CPU load", tile_h, |ui| {
                let lines = [
                    s.cpu_mhz.map(|m| format!("{:.2} GHz", m as f32 / 1000.0)).unwrap_or_default(),
                    package_w.map(|w| format!("{w:.1} W package")).unwrap_or_default(),
                ];
                big_value(ui, s.cpu_load_pct.map(|l| format!("{l:.0}")), "%", CYAN(), &lines);
                load_graph(ui, &hist(&self.hist_cpu_load));
            });
            card_sized(&mut c[1], "Discrete GPU load", tile_h, |ui| match s.dgpu() {
                Some(d) if d.awake => {
                    let lines = [
                        d.clock_mhz.map(clock_text).unwrap_or_default(),
                        d.power_w.map(|w| format!("{w:.1} W")).unwrap_or_default(),
                    ];
                    big_value(ui, d.busy_pct.map(|b| b.to_string()), "%", CYAN(), &lines);
                    load_graph(ui, &hist(&self.hist_dgpu_load));
                }
                Some(_) => {
                    big_value(ui, Some("Asleep".into()), "", DIM(), &["saving power".into()]);
                    load_graph(ui, &hist(&self.hist_dgpu_load));
                }
                None => {
                    let why = if s.gfx_mode() == GfxMode::Eco { "Switched off (Eco)." } else { "Not found right now." };
                    ui.label(RichText::new(why).color(DIM()));
                }
            });
            card_sized(&mut c[2], "Integrated GPU load", tile_h, |ui| match s.igpu() {
                Some(i) => {
                    let lines = [
                        i.clock_mhz.map(clock_text).unwrap_or_default(),
                        i.power_w.map(|w| format!("{w:.1} W package")).unwrap_or_default(),
                    ];
                    big_value(ui, i.busy_pct.map(|b| b.to_string()), "%", CYAN(), &lines);
                    load_graph(ui, &hist(&self.hist_igpu_load));
                }
                None => {
                    ui.label(RichText::new("Not found.").color(DIM()));
                }
            });
        });
        ui.add_space(gap);

        // Row 2: temperatures and memory.
        let temp_tile = |ui: &mut Ui, now: Option<f32>, h: &[f32]| {
            let peak = h.iter().copied().fold(f32::MIN, f32::max);
            let lines = if h.is_empty() { vec![] } else { vec![format!("peak {peak:.0}°C")] };
            let color = now.map(temp_color).unwrap_or(DIM());
            big_value(ui, now.map(|t| format!("{t:.0}")), "°C", color, &lines);
            let gh = (ui.available_height() - 4.0).max(36.0);
            temp_graph(ui, h, color, gh);
        };
        ui.columns(3, |c| {
            card_sized(&mut c[0], "CPU temperature", tile_h, |ui| {
                temp_tile(ui, s.cpu_temp_c, &hist(&self.hist_cpu_temp));
            });
            card_sized(&mut c[1], gpu_title, tile_h, |ui| {
                temp_tile(ui, gpu.as_ref().and_then(|x| x.temp_c), &hist(&self.hist_gpu_temp));
            });
            card_sized(&mut c[2], "Memory", tile_h, |ui| {
                // Two label rows (~30 px each with spacing) and two graphs share the height.
                let half = ((ui.available_height() - 92.0) / 2.0).max(18.0);
                stat_row(ui, "System RAM", match (s.mem_used_gb, s.mem_total_gb) {
                    (Some(u), Some(t)) => format!("{u:.1} / {t:.0} GB"),
                    _ => "–".into(),
                });
                sparkline(ui, &hist(&self.hist_mem), 0.0, 100.0, level_color(self.hist_mem.back().copied().unwrap_or(0.0) / 100.0, GREEN()), half);
                stat_row(ui, "GPU memory", match gpu.as_ref().map(|x| (x.vram_used_mb, x.vram_total_mb)) {
                    Some((Some(u), Some(t))) => format!("{:.1} / {:.1} GB", u as f32 / 1024.0, t as f32 / 1024.0),
                    _ => "–".into(),
                });
                sparkline(ui, &hist(&self.hist_vram), 0.0, 100.0, level_color(self.hist_vram.back().copied().unwrap_or(0.0) / 100.0, GREEN()), half);
            });
        });
        ui.add_space(gap);

        // Row 3: fans, storage, battery.
        ui.columns(3, |c| {
            card_sized(&mut c[0], "Fans", tile_h, |ui| {
                let n = s.fans.len().max(1) as f32;
                // Each fan: a label row (~30 px) plus spacing, then its graph.
                let h = ((ui.available_height() - 50.0 * n) / n).max(18.0);
                for (i, (label, rpm)) in s.fans.iter().enumerate() {
                    stat_row(ui, &fan_label(label), format!("{rpm} rpm"));
                    if let Some(q) = self.hist_fans.get(i) {
                        sparkline(ui, &hist(q), 0.0, 6500.0, CYAN(), h);
                    }
                }
            });
            card_sized(&mut c[1], "Storage", tile_h, |ui| {
                for d in &s.disks {
                    let used = d.used_frac();
                    let color = level_color(used, GREEN());
                    stat_row(
                        ui,
                        &d.name,
                        format!("{} free of {}", crate::hw::human_bytes(d.avail), crate::hw::human_bytes(d.size)),
                    );
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), egui::Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(rect, 4.0, CARD_HI());
                    p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(rect.width() * used, rect.height())), 4.0, color);
                    ui.add_space(4.0);
                }
                if s.disks.is_empty() {
                    ui.label(RichText::new("Reading drives…").color(DIM()));
                }
            });
            card_sized(&mut c[2], "Battery", tile_h, |ui| {
                let b = s.battery_pct.unwrap_or(0) as f32;
                let color = battery_color(b);
                let sub = battery_eta(&s, batt_avg).unwrap_or_else(|| match (s.battery_power_w, s.battery_status.as_str()) {
                    (Some(w), "Discharging") if w > 0.1 => format!("−{w:.1} W"),
                    (Some(w), "Charging") if w > 0.1 => format!("+{w:.1} W"),
                    (_, st) => st.to_string(),
                });
                big_value(ui, Some(format!("{b:.0}")), "%", color, &[sub]);
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
                let p = ui.painter();
                p.rect_filled(rect, 5.0, CARD_HI());
                p.rect_filled(egui::Rect::from_min_size(rect.min, vec2(rect.width() * b / 100.0, rect.height())), 5.0, color);
                ui.add_space(4.0);
                let charger = match s.on_ac {
                    Some(true) => "plugged in".to_string(),
                    Some(false) => "unplugged".to_string(),
                    None => "–".to_string(),
                };
                let limit = s.charge_limit.map(|c| format!("{c}%")).unwrap_or("–".into());
                let health = s.battery_health_pct.map(|h| format!("{h:.0}%")).unwrap_or("–".into());
                // Rows sit about 32 px apart and the last needs ~22 px of text: when the tile is short, fold the details together
                // rather than let the last ones be cut off.
                let room = ui.available_height();
                if room >= 2.0 * 32.0 + 22.0 {
                    stat_row(ui, "Charger", charger);
                    stat_row(ui, "Charge limit", limit);
                    stat_row(ui, "Health", health);
                } else if room >= 32.0 + 22.0 {
                    stat_row(ui, "Charger", charger);
                    stat_row(ui, "Limit · health", format!("{limit} · {health}"));
                } else {
                    stat_row(ui, "Charger · limit · health", format!("{charger} · {limit} · {health}"));
                }
            });
        });
    }

    fn performance(&mut self, ui: &mut Ui) {
        page_title(ui, "Performance", "Choose how much power the laptop may use and how hard it cools.");
        self.profile_tiles(ui, false);
        ui.add_space(8.0);
        self.power_limits_card(ui);
        ui.add_space(4.0);
        card(ui, "Right now", |ui| {
            let s = &self.snap;
            ui.columns(3, |c| {
                stat_row(&mut c[0], "CPU", s.cpu_temp_c.map(|t| format!("{t:.0}°C")).unwrap_or("–".into()));
                stat_row(&mut c[1], "Clock", s.cpu_mhz.map(|m| format!("{m} MHz")).unwrap_or("–".into()));
                stat_row(&mut c[2], "Load", s.cpu_load_pct.map(|l| format!("{l:.0}%")).unwrap_or("–".into()));
            });
        });
    }

    /// The active mode's power limits. Each mode (plugged in and on battery separately) keeps
    /// its own, so the sliders move when the mode changes. A slider applies when let go; the
    /// small mark under it is the stock setting.
    fn power_limits_card(&mut self, ui: &mut Ui) {
        let s = &self.snap;
        let limits: Vec<crate::hw::Attr> = s.power_limits().into_iter().cloned().collect();
        // What each slider shows for this mode (see `Snapshot::power_limit_shown`).
        let now: BTreeMap<String, i64> =
            limits.iter().filter_map(|a| Some((a.name.clone(), s.power_limit_shown(a)?))).collect();
        let profile = s.profile.clone().unwrap_or_default();
        let mode = profile_title(&profile);
        let on_ac = s.on_ac.unwrap_or(true);
        let tuned = s.tuning_now().is_some_and(|t| t.enabled);
        let fan_curve_on = s.cpu_curve_enabled == Some(true);
        // Values just sent stay on the sliders until the readings show them.
        if let Some((t, sent)) = &self.limits_sent {
            let caught_up = sent.iter().all(|(n, v)| now.get(n) == Some(v));
            if caught_up || t.elapsed() > Duration::from_secs(6) {
                self.limits_sent = None;
            }
        }
        let title = format!("Power limits · {mode}, {}", if on_ac { "plugged in" } else { "on battery" });
        let mut send: Vec<(String, i64)> = Vec::new();
        let mut to_stock = false;
        card(ui, &title, |ui| {
            if limits.is_empty() {
                ui.label(RichText::new("This laptop's firmware doesn't offer power limits to change.").color(DIM()));
                return;
            }
            ui.label(
                RichText::new(
                    "Each mode keeps its own limits, plugged in and on battery. Lower numbers run cooler, quieter \
                     and longer on battery; higher numbers let games and heavy work go faster.",
                )
                .color(DIM()),
            );
            ui.add_space(2.0);
            let status = if tuned {
                format!("{mode} uses your limits.")
            } else {
                format!(
                    "{mode} uses the laptop's stock limits: the mode's own built-in limits apply. The laptop doesn't \
                     report those numbers, so the sliders sit on their stock marks. Moving one gives {mode} limits of its own."
                )
            };
            ui.label(RichText::new(status).color(if tuned { profile_color(&profile) } else { DIM() }));
            ui.add_space(10.0);

            let label_w = 330.0_f32.min(ui.available_width() * 0.4);
            let value_w = 150.0;
            for a in &limits {
                let (lo, hi) = (a.min.unwrap_or(5), a.max.unwrap_or(150));
                let unit = power_limit_unit(&a.name);
                let cur = now.get(&a.name).copied().unwrap_or(hi);
                let shown = self
                    .limits_sent
                    .as_ref()
                    .and_then(|(_, m)| m.get(&a.name).copied())
                    .unwrap_or(cur);
                let mut v = self.attr_edit.get(&a.name).copied().unwrap_or(shown);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(vec2(label_w, 44.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(label_w);
                        ui.label(RichText::new(attr_title(&a.name)).color(TEXT()).size(16.0));
                        ui.label(RichText::new(power_limit_hint(&a.name)).color(DIM()).size(12.5));
                    });
                    let slider_w = (ui.available_width() - value_w - 16.0).max(120.0);
                    ui.spacing_mut().slider_width = slider_w;
                    let r = ui.add(egui::Slider::new(&mut v, lo..=hi).show_value(false));
                    // The stock mark under the rail.
                    if let Some(d) = a.default.filter(|_| hi > lo) {
                        let rect = r.rect;
                        let range = rect.x_range().shrink(rect.height() / 2.5);
                        let t = (d - lo) as f32 / (hi - lo) as f32;
                        let x = range.min + (range.max - range.min) * t.clamp(0.0, 1.0);
                        let y = rect.center().y + 7.0;
                        ui.painter().line_segment(
                            [egui::pos2(x, y), egui::pos2(x, y + 7.0)],
                            egui::Stroke::new(2.0, DIM()),
                        );
                    }
                    if r.changed() {
                        self.attr_edit.insert(a.name.clone(), v);
                    }
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.attr_edit.remove(&a.name);
                        if v != cur {
                            send.push((a.name.clone(), v));
                        }
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("{v} {unit}")).color(TEXT()).size(18.0).strong());
                        let note = match a.default {
                            Some(d) if d == v => "stock".to_string(),
                            Some(d) => format!("stock {d} {unit}"),
                            None => String::new(),
                        };
                        ui.label(RichText::new(note).color(DIM()).size(12.5));
                    });
                });
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                ui.add_enabled_ui(tuned, |ui| {
                    if ui.button(format!("Back to stock for {mode}")).clicked() {
                        to_stock = true;
                    }
                });
                ui.label(
                    RichText::new("Changing a limit gives this mode its own; the other modes keep theirs.")
                        .color(DIM())
                        .size(12.5),
                );
            });
        });

        if to_stock {
            let back: Vec<(String, i64)> = limits.iter().filter_map(|a| Some((a.name.clone(), a.default?))).collect();
            self.attr_edit.clear();
            self.limits_sent = Some((Instant::now(), back.into_iter().collect()));
            self.run(ctl::power_limits(&profile, &[], tuned, fan_curve_on, true));
        } else if !send.is_empty() {
            let mut map = self.limits_sent.take().map(|(_, m)| m).unwrap_or_default();
            map.extend(send.iter().cloned());
            self.limits_sent = Some((Instant::now(), map));
            self.run(ctl::power_limits(&profile, &send, tuned, fan_curve_on, false));
        }
    }

    /// Fans: pick Silent, Balanced or Turbo (this switches the laptop's mode), see that
    /// mode's curves, drag a point to make them yours, or reset them to stock.
    fn fans(&mut self, ui: &mut Ui) {
        page_title(
            ui,
            "Fans",
            "Pick a mode to use it. Drag a point to adjust its fan curve; it's applied when you let go.",
        );
        let mut choices = self.snap.profile_choices.clone();
        choices.retain(|c| c != "custom");
        if choices.is_empty() {
            choices = vec!["quiet".into(), "balanced".into(), "performance".into()];
        }
        let current = self.snap.profile.clone().unwrap_or_default();

        // The three modes; each shows whether its fan curve is stock or yours.
        let n = choices.len().max(1) as f32;
        let w = ((ui.available_width() - 12.0 * (n - 1.0)) / n).max(140.0);
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for c in &choices {
                let icon = match c.as_str() {
                    "quiet" | "low-power" => "🌙",
                    "performance" => "🚀",
                    _ => "⚖",
                };
                let mine = self.settings.curves.get(c).is_some_and(|p| p.enabled);
                let desc = if mine { "Custom fan curve (yours)" } else { "Stock fan curve" };
                if mode_tile(ui, w, icon, profile_title(c), desc, *c == current, profile_color(c)).clicked() && *c != current {
                    pick = Some(c.clone());
                }
            }
        });
        if let Some(c) = pick {
            self.run(ctl::set_profile(&c));
        }
        if current.is_empty() {
            ui.label(RichText::new("The laptop didn't report its performance mode.").color(AMBER()));
            return;
        }

        // The stock curve comes from the laptop once per mode (see `fetch_stock`).
        let stock = self.settings.stock.get(&current).cloned();
        if stock.is_none()
            && self.stock_fetching.is_none()
            && self.shot.is_none()
            && self.tools.checked
            && self.tools.asusctl.is_some()
        {
            self.fetch_stock(&current);
        }
        let mine = self.settings.curves.get(&current).is_some_and(|p| p.enabled);
        // Without a stock reading yet, show what the fans run now.
        // Every curve is edited on the same fixed temperature columns (30°, 40° … 100°).
        let shown_stock = stock
            .clone()
            .map(|s| (s.cpu, s.gpu))
            .or(self.snap.cpu_curve.zip(self.snap.gpu_curve))
            .map(|(c, g)| (settings::on_columns(&c), settings::on_columns(&g)));

        let cpu_temp = self.snap.cpu_temp_c;
        let gpu_temp = self.snap.dgpu().and_then(|g| g.temp_c);
        let fans = self.snap.fans.clone();
        let mut changed = false;
        {
            let pc = self.settings.curves_for(&current);
            if !mine {
                // Editing starts from stock.
                if let Some((c, g)) = shown_stock {
                    pc.cpu = c;
                    pc.gpu = g;
                }
            } else if !settings::is_on_columns(&pc.cpu) || !settings::is_on_columns(&pc.gpu) {
                // A custom curve from before the fixed columns: move it onto them once and
                // send it again, so what's shown is what the fans run.
                pc.cpu = settings::on_columns(&pc.cpu);
                pc.gpu = settings::on_columns(&pc.gpu);
                changed = true;
            }
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                card(&mut cols[0], "CPU fan", |ui| {
                    if let Some((_, rpm)) = fans.first() {
                        ui.label(RichText::new(format!("{rpm} rpm")).size(18.0).strong());
                    }
                    changed |= curve_editor(ui, "cpu_curve", &mut pc.cpu, cpu_temp, ACCENT(), mine);
                });
                card(&mut cols[1], "GPU fan", |ui| {
                    if let Some((_, rpm)) = fans.get(1) {
                        ui.label(RichText::new(format!("{rpm} rpm")).size(18.0).strong());
                    }
                    changed |= curve_editor(ui, "gpu_curve", &mut pc.gpu, gpu_temp.or(cpu_temp), CYAN(), mine);
                });
            });
            if changed {
                pc.enabled = true;
            }
        }
        if changed {
            self.settings.save();
            // Sent once the mouse button is released (see `send_pending_fans`).
            self.fan_pending = Some((current.clone(), Instant::now()));
        }

        ui.add_space(4.0);
        let mode = profile_title(&current);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(mine, |ui| {
                if ui
                    .button(format!("Reset {mode} to stock"))
                    .on_hover_text("Brings back the laptop's own fan curve for this mode.")
                    .clicked()
                {
                    let hw = self.snap.curve_hwmon.clone();
                    let pc = self.settings.curves_for(&current);
                    pc.enabled = false;
                    if let Some(s) = &stock {
                        pc.cpu = settings::on_columns(&s.cpu);
                        pc.gpu = settings::on_columns(&s.gpu);
                    }
                    self.settings.save();
                    self.fan_pending = None;
                    self.run(ctl::fan_curves_off(&current, true, hw.as_deref()));
                }
            });
            let note = if self.stock_fetching.is_some() {
                "Reading the stock curve from the laptop…".to_string()
            } else if mine {
                format!("{mode} uses your curve.")
            } else {
                format!("{mode} uses the laptop's stock curve. Drag a point to make it yours.")
            };
            ui.label(RichText::new(note).color(DIM()));
            if self.stock_fetching.is_some() {
                ui.spinner();
            }
        });
        ui.label(
            RichText::new("Silent, Balanced and Turbo also set the power limits, like on the Performance page.")
                .color(DIM())
                .size(12.5),
        );
    }

    /// Read a mode's stock fan curves once: asusctl resets the stored curve to the factory one
    /// (`fan-curve --default`, active mode only), we read it, and put your curve back if you
    /// use one. This also clears leftovers from older versions.
    fn fetch_stock(&mut self, mode: &str) {
        self.stock_fetching = Some(mode.to_string());
        let mode = mode.to_string();
        let tools = self.tools.clone();
        let mine = self.settings.curves.get(&mode).filter(|p| p.enabled).cloned();
        let hw = self.snap.curve_hwmon.clone();
        let slot = self.fan_reload.clone();
        let c = self.ctx.clone();
        std::thread::spawn(move || {
            let reset = ctl::run_action(ctl::reset_fan_curves(&mode, &mode, hw.as_deref()), &tools);
            let curves = if reset.ok { ctl::read_fan_curves(&mode) } else { None };
            if let Some(p) = mine {
                let _ = ctl::run_action(ctl::apply_fan_curves(&mode, &p.cpu, &p.gpu, true, hw.as_deref()), &tools);
            }
            if let Ok(mut g) = slot.lock() {
                *g = Some((mode, curves));
            }
            c.request_repaint();
        });
    }

    /// Apply a dragged curve once the mouse button is up.
    fn send_pending_fans(&mut self) {
        if self.shot.is_some() {
            return; // a screenshot run never changes the fans
        }
        let Some((mode, at)) = self.fan_pending.clone() else { return };
        let held = self.ctx.input(|i| i.pointer.any_down());
        if held || at.elapsed() < Duration::from_millis(150) {
            self.ctx.request_repaint_after(Duration::from_millis(150));
            return;
        }
        self.fan_pending = None;
        let Some(pc) = self.settings.curves.get(&mode).cloned() else { return };
        let active = self.snap.profile.as_deref() == Some(mode.as_str());
        let hw = self.snap.curve_hwmon.clone();
        self.run(ctl::apply_fan_curves(&mode, &pc.cpu, &pc.gpu, active, hw.as_deref()));
    }

    fn battery(&mut self, ui: &mut Ui) {
        page_title(ui, "Battery", "Stop charging early to keep the battery healthy for longer.");
        let s = self.snap.clone();
        ui.columns(2, |cols| {
            card(&mut cols[0], "Battery", |ui| {
                ui.horizontal(|ui| {
                    let b = s.battery_pct.unwrap_or(0) as f32;
                    gauge(ui, 150.0, b / 100.0, &format!("{b:.0}"), "%", &s.battery_status, battery_color(b));
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        stat_row(ui, "Status", s.battery_status.clone());
                        stat_row(ui, "Power", s.battery_power_w.map(|w| format!("{w:.1} W")).unwrap_or("–".into()));
                        stat_row(ui, "Charger", match s.on_ac {
                            Some(true) => "plugged in".into(),
                            Some(false) => "unplugged".into(),
                            None => "–".into(),
                        });
                        stat_row(ui, "Health", s.battery_health_pct.map(|h| format!("{h:.0}% of new")).unwrap_or("–".into()));
                        stat_row(ui, "Cycles", s.battery_cycles.map(|c| c.to_string()).unwrap_or("–".into()));
                    });
                });
            });
            card(&mut cols[1], "Charge limit", |ui| {
                let cur = s.charge_limit.unwrap_or(100);
                let v = self.charge_edit.get_or_insert(cur);
                ui.label(RichText::new(format!("{v}%")).size(40.0).strong().color(TEXT()));
                ui.label(RichText::new(format!("Charging stops at this level. Now set to {cur}%.")).color(DIM()));
                ui.add_space(6.0);
                ui.spacing_mut().slider_width = ui.available_width() - 70.0;
                ui.add(egui::Slider::new(v, 20..=100).suffix("%"));
                ui.horizontal(|ui| {
                    for (label, val) in [("Max lifespan 60%", 60u8), ("Balanced 80%", 80), ("Full 100%", 100)] {
                        if ui.button(label).clicked() {
                            *v = val;
                        }
                    }
                });
                ui.add_space(6.0);
                let target = *v;
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(target != cur, |ui| {
                        if accent_button(ui, "Apply limit").clicked() {
                            self.run(ctl::set_charge_limit(target, s.charge_limit_path.clone()));
                        }
                    });
                    if ui
                        .button("Charge to 100% once")
                        .on_hover_text("Fills the battery fully this one time (for a trip), then goes back to your limit.")
                        .clicked()
                    {
                        self.run(ctl::one_shot_charge());
                    }
                });
            });
        });
        self.battery_automation(ui);
    }

    /// "On battery": what changes by itself when the charger is unplugged, and comes back
    /// when it's plugged in again.
    fn battery_automation(&mut self, ui: &mut Ui) {
        use crate::power::Item;
        let mut changes: Vec<(Item, bool)> = Vec::new();
        let mut calm_changed = false;
        let has_overdrive = self.snap.attr("panel_overdrive").is_some();
        let st = &mut self.settings;
        card(ui, "On battery", |ui| {
            ui.label(
                RichText::new("Switched on here, these change by themselves when the charger is unplugged, and go back when it's plugged in.")
                    .color(DIM()),
            );
            ui.add_space(6.0);
            let row = |ui: &mut Ui, v: &mut bool, label: &str, hint: &str| -> bool {
                ui.horizontal(|ui| {
                    let c = switch(ui, v).changed();
                    ui.label(RichText::new(label).color(TEXT()));
                    if !hint.is_empty() {
                        ui.label(RichText::new(hint).color(DIM()));
                    }
                    c
                })
                .inner
            };
            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                ui.label(RichText::new("PERFORMANCE AND GRAPHICS").size(12.0).color(DIM()));
                if row(ui, &mut st.battery_silent, "Performance mode Silent", "") {
                    changes.push((Item::Profile, st.battery_silent));
                }
                if row(ui, &mut st.battery_silent_fans, "Silent fan curves", "(in whichever mode is on)") {
                    changes.push((Item::Fans, st.battery_silent_fans));
                }
                if row(ui, &mut st.battery_eco, "Graphics Eco", "(not while HDMI is in use)") {
                    changes.push((Item::Gfx, st.battery_eco));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("THIS WINDOW").size(12.0).color(DIM()));
                // Only the window's own drawing: nothing on the laptop changes.
                calm_changed = row(ui, &mut st.battery_calm, "Calm it down", "(Neon's moving colours stop)");

                let ui = &mut cols[1];
                ui.label(RichText::new("SCREEN AND KEYBOARD").size(12.0).color(DIM()));
                if row(ui, &mut st.battery_60hz, "Screen at 60 Hz", "") {
                    changes.push((Item::Refresh, st.battery_60hz));
                }
                if has_overdrive && row(ui, &mut st.battery_overdrive_off, "Panel overdrive off", "") {
                    changes.push((Item::Overdrive, st.battery_overdrive_off));
                }
                if row(ui, &mut st.battery_backlight, "Screen brightness", "") {
                    changes.push((Item::Backlight, st.battery_backlight));
                }
                ui.add_enabled_ui(st.battery_backlight, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(66.0);
                        ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(120.0);
                        let r = ui.add(egui::Slider::new(&mut st.battery_backlight_pct, 10..=100).suffix("%"));
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            changes.push((Item::Backlight, true));
                        }
                    });
                });
                // Off and dimmed can't both apply: switching one on switches the other off.
                if row(ui, &mut st.kbd_off_on_battery, "Keyboard light off", "") {
                    if st.kbd_off_on_battery {
                        st.battery_kbd_dim = false;
                    }
                    changes.push((Item::Keyboard, st.kbd_off_on_battery || st.battery_kbd_dim));
                }
                if row(ui, &mut st.battery_kbd_dim, "Dim the keyboard light", "") {
                    if st.battery_kbd_dim {
                        st.kbd_off_on_battery = false;
                    }
                    changes.push((Item::Keyboard, st.kbd_off_on_battery || st.battery_kbd_dim));
                }
                ui.add_enabled_ui(st.battery_kbd_dim && !st.kbd_off_on_battery, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(66.0);
                        ui.spacing_mut().slider_width = (ui.available_width() - 80.0).max(120.0);
                        let r = ui.add(
                            egui::Slider::new(&mut st.battery_kbd_level, 1..=3)
                                .show_value(true)
                                .custom_formatter(|v, _| ["Low", "Medium", "High"][(v as usize).clamp(1, 3) - 1].to_string())
                                .custom_parser(|s| match s.trim().to_lowercase().as_str() {
                                    "low" => Some(1.0),
                                    "medium" | "med" => Some(2.0),
                                    "high" => Some(3.0),
                                    _ => None,
                                }),
                        );
                        if r.drag_stopped() || (r.changed() && !r.dragged()) {
                            changes.push((Item::Keyboard, true));
                        }
                    });
                });
            });
        });
        if calm_changed {
            self.settings.save();
        }
        if changes.is_empty() {
            return;
        }
        self.settings.save();
        // Already on battery: act now, the same way unplugging would.
        if self.snap.on_ac == Some(false) && self.tools.checked && self.shot.is_none() {
            for (item, on) in changes {
                let c = self.ctx.clone();
                crate::power::toggle(item, on, &self.tools, self.tx.clone(), move || c.request_repaint());
            }
        }
    }

    fn lighting(&mut self, ui: &mut Ui) {
        page_title(ui, "Lighting", "Keyboard colour, effect and brightness.");
        let t = self.started.elapsed().as_secs_f32();
        let a = self.settings.aura.clone();
        let lvl = self.snap.kbd_brightness.unwrap_or(3);
        let max = self.snap.kbd_max.unwrap_or(3).max(1);
        // Keep a dim preview even when the backlight is off, so the effect can still be judged.
        let bright = if lvl == 0 { 0.3 } else { 0.4 + 0.6 * lvl as f32 / max as f32 };
        let spd = [0.5, 1.0, 2.0][a.speed.min(2) as usize];
        let c1 = Color32::from_rgb(a.color1[0], a.color1[1], a.color1[2]);
        let c2 = Color32::from_rgb(a.color2[0], a.color2[1], a.color2[2]);
        let dir = if a.direction == "left" { 1.0 } else { -1.0 };
        card(ui, "Preview", |ui| {
            ui.vertical_centered(|ui| {
                keyboard_preview(
                    ui,
                    |x| match a.mode {
                        AuraMode::Static => c1,
                        AuraMode::Breathe => {
                            let ph = (t * spd * 0.5) % 2.0;
                            let base = if ph < 1.0 { c1 } else { c2 };
                            let k = (ph.fract() * std::f32::consts::PI).sin();
                            lerp_color(Color32::BLACK, base, k)
                        }
                        AuraMode::Pulse => lerp_color(Color32::BLACK, c1, ((t * 3.0).sin() * 0.5 + 0.5).powf(2.0)),
                        AuraMode::RainbowCycle => hsv(t * 0.1 * spd, 1.0, 1.0),
                        AuraMode::RainbowWave => hsv(x * 0.8 + dir * t * 0.25 * spd, 1.0, 1.0),
                    },
                    bright,
                );
            });
        });
        if a.mode != AuraMode::Static {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        ui.columns(2, |cols| {
            let a = &mut self.settings.aura;
            let before = a.clone();
            card(&mut cols[0], "Effect", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for m in AuraMode::ALL {
                        let sel = a.mode == m;
                        let b = egui::Button::new(RichText::new(m.title()).color(if sel { Color32::WHITE } else { TEXT() }))
                            .fill(if sel { ACCENT() } else { CARD_HI() })
                            .min_size(vec2(110.0, 32.0));
                        if ui.add(b).clicked() {
                            a.mode = m;
                        }
                    }
                });
                ui.add_space(6.0);
                if a.mode.uses_color() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Colour").color(DIM()));
                        ui.color_edit_button_srgb(&mut a.color1);
                        if a.mode == AuraMode::Breathe {
                            ui.label(RichText::new("Second colour").color(DIM()));
                            ui.color_edit_button_srgb(&mut a.color2);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        for c in [
                            [255, 32, 64],
                            [255, 120, 0],
                            [255, 220, 0],
                            [40, 230, 90],
                            [0, 200, 255],
                            [40, 80, 255],
                            [170, 60, 255],
                            [255, 255, 255],
                        ] {
                            let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::click());
                            ui.painter().circle_filled(r.center(), 10.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if a.color1 == c {
                                ui.painter().circle_stroke(r.center(), 12.0, egui::Stroke::new(2.0, Color32::WHITE));
                            }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                a.color1 = c;
                            }
                        }
                    });
                }
                if a.mode.uses_speed() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Speed").color(DIM()));
                        let mut s = a.speed as usize;
                        if segmented(ui, &["Slow", "Medium", "Fast"], &mut s) {
                            a.speed = s as u8;
                        }
                    })
                    .response
                    .on_hover_text("The keyboard's firmware has these three speeds only.");
                }
                if a.mode == AuraMode::RainbowWave {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Direction").color(DIM()));
                        let opts = ["left", "right", "up", "down"];
                        let mut d = opts.iter().position(|o| *o == a.direction).unwrap_or(1);
                        if segmented(ui, &["Left", "Right", "Up", "Down"], &mut d) {
                            a.direction = opts[d].into();
                        }
                    });
                }
                ui.add_space(4.0);
                ui.label(RichText::new("Changes are sent to the keyboard as you pick them.").color(DIM()).size(12.5));
            });
            let mut level = lvl as usize;
            let mut set_level = None;
            let mut apply_power = false;
            let mut go_battery_page = false;
            card(&mut cols[1], "Brightness", |ui| {
                if segmented(ui, &["Off", "Low", "Medium", "High"], &mut level) {
                    set_level = Some(level as u8);
                }
                ui.add_space(4.0);
                go_battery_page = ui
                    .link(RichText::new("Dim or turn it off on battery → Battery page").color(DIM()).size(12.5))
                    .clicked();
            });
            card(&mut cols[1], "When is the keyboard lit", |ui| {
                for (label, v) in [
                    ("While using the laptop", &mut a.awake),
                    ("While starting up", &mut a.boot),
                    ("While asleep", &mut a.sleep),
                    ("While shutting down", &mut a.shutdown),
                ] {
                    ui.horizontal(|ui| {
                        if switch(ui, v).changed() {
                            apply_power = true;
                        }
                        ui.label(RichText::new(label).color(TEXT()));
                    });
                }
            });
            let mut anime_on = self.settings.anime_on;
            let mut anime_b = self.settings.anime_brightness as usize;
            let mut anime_actions = Vec::new();
            let has_matrix = self.snap.has_lid_matrix;
            if has_matrix {
            card(&mut cols[1], "Lid matrix display", |ui| {
                ui.label(RichText::new("The dot-matrix display on the lid.").color(DIM()));
                ui.horizontal(|ui| {
                    if switch(ui, &mut anime_on).changed() {
                        anime_actions.push(ctl::anime_enable(anime_on));
                    }
                    ui.label("Display on");
                });
                if segmented(ui, &["Off", "Low", "Medium", "High"], &mut anime_b) {
                    anime_actions.push(ctl::anime_brightness(anime_b as u8));
                }
            });
            }
            let aura = a.clone();
            if !aura.same_effect(&before) {
                // Sent from `send_pending_aura` once the edit settles.
                self.aura_pending = Some(Instant::now());
            }
            if apply_power {
                self.settings.save();
                self.run(ctl::set_aura_power(&aura));
            }
            if let Some(l) = set_level {
                self.run(ctl::set_kbd_brightness(l));
            }
            if go_battery_page {
                self.page = Page::Battery;
            }
            if !anime_actions.is_empty() {
                self.settings.anime_on = anime_on;
                self.settings.anime_brightness = anime_b as u8;
                self.settings.save();
                for act in anime_actions {
                    self.run(act);
                }
            }
        });
    }

    fn gfx_mode(&self) -> GfxMode {
        self.snap.gfx_mode()
    }

    fn switch_gfx(&mut self, m: GfxMode) {
        self.run(ctl::set_gfx(m, &self.snap.gfx_state()));
    }

    /// Pick a graphics mode: right away, or through the restart question when it needs one.
    fn pick_gfx(&mut self, m: GfxMode) {
        let mux_now = self.snap.attr("gpu_mux_mode").and_then(|a| a.current);
        let now = self.gfx_mode();
        if m == self.snap.gfx_pending.unwrap_or(now) {
            return;
        }
        if m == now {
            // Back to the running mode: takes back the switch waiting for the restart.
            self.switch_gfx(m);
        } else if m == GfxMode::Ultimate || mux_now == Some(0) {
            self.page = Page::Graphics;
            self.gfx_confirm = Some(m);
        } else {
            self.switch_gfx(m);
        }
    }

    /// The "this needs a restart" questions, drawn over the whole window.
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(m) = self.gfx_confirm {
            let (title, body) = gfx_restart_text(m);
            let mut choice = None;
            let mut cancel = false;
            let resp = egui::Modal::new(egui::Id::unique("gfx-confirm")).show(ctx, |ui| {
                ui.set_width(480.0);
                ui.label(RichText::new(title).size(20.0).strong().color(TEXT()));
                ui.add_space(6.0);
                ui.label(RichText::new(body).color(DIM()));
                ui.add_space(4.0);
                ui.label(RichText::new("Save your work in other apps before restarting.").color(AMBER()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Restart Now").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Restart Later").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
            if cancel || resp.should_close() {
                self.gfx_confirm = None;
            }
            if let Some(now) = choice {
                self.gfx_confirm = None;
                self.restart_after_gfx = now;
                self.switch_gfx(m);
            }
        }
        if self.restart_confirm {
            let mut go = false;
            let mut close = false;
            let resp = egui::Modal::new(egui::Id::unique("restart-confirm")).show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Restart the laptop now?").size(20.0).strong().color(TEXT()));
                ui.add_space(6.0);
                ui.label(RichText::new("The new graphics mode starts after the restart.").color(DIM()));
                ui.label(RichText::new("Save your work in other apps first.").color(AMBER()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Restart Now").clicked() {
                        go = true;
                    }
                    if ui.button("Not now").clicked() {
                        close = true;
                    }
                });
            });
            if close || resp.should_close() {
                self.restart_confirm = false;
            }
            if go {
                self.restart_confirm = false;
                self.run(ctl::restart_laptop());
            }
        }
        if self.shutdown_confirm {
            let mut go = false;
            let mut close = false;
            let resp = egui::Modal::new(egui::Id::unique("shutdown-confirm")).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(RichText::new("Shut the laptop down now?").size(20.0).strong().color(TEXT()));
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Wait about 10 seconds after it turns off, then press the power button.").color(DIM()),
                );
                ui.label(RichText::new("Save your work in other apps first.").color(AMBER()));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if accent_button(ui, "Shut down now").clicked() {
                        go = true;
                    }
                    if ui.button("Not now").clicked() {
                        close = true;
                    }
                });
            });
            if close || resp.should_close() {
                self.shutdown_confirm = false;
            }
            if go {
                self.shutdown_confirm = false;
                self.run(ctl::shut_down_laptop());
            }
        }
    }

    fn graphics(&mut self, ui: &mut Ui) {
        page_title(ui, "Graphics", "Choose which graphics chips are used.");
        let cur = self.gfx_mode();
        // With a switch waiting for the restart, that one is the chosen tile.
        let pending = self.snap.gfx_pending;
        let chosen = pending.unwrap_or(cur);
        let w = (ui.available_width() - 24.0) / 3.0;
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            for (m, icon, desc) in [
                (GfxMode::Eco, "🍃", "Turns the discrete GPU off. Longest battery life."),
                (GfxMode::Standard, "⚡", "The discrete GPU wakes up only when a game or app needs it."),
                (GfxMode::Ultimate, "🎮", "The screen is wired straight to the discrete GPU. Needs a restart."),
            ] {
                let desc = if pending == Some(m) {
                    "Chosen: starts after the restart."
                } else if pending.is_some() && m == cur {
                    "In use until the restart. Pick it to stay here."
                } else {
                    desc
                };
                if mode_tile(ui, w, icon, m.title(), desc, chosen == m, if m == GfxMode::Ultimate { ACCENT() } else if m == GfxMode::Eco { GREEN() } else { CYAN() }).clicked()
                    && chosen != m
                {
                    pick = Some(m);
                }
            }
        });
        if let Some(m) = pick {
            // Going into or out of Ultimate flips the MUX, which only takes effect after a restart.
            self.pick_gfx(m);
        }
        ui.add_space(8.0);
        let s = self.snap.clone();
        ui.columns(2, |cols| {
            for (i, (title, g)) in [("Integrated GPU", s.igpu()), ("Discrete GPU", s.dgpu())].into_iter().enumerate() {
                card(&mut cols[i], title, |ui| match g {
                    None => {
                        ui.label(RichText::new("Not present (turned off or not found)").color(DIM()));
                    }
                    Some(g) => {
                        stat_row(ui, "State", if g.awake { "active".into() } else { "asleep".into() });
                        stat_row(ui, "Temperature", g.temp_c.map(|t| format!("{t:.0}°C")).unwrap_or("–".into()));
                        stat_row(ui, "Busy", g.busy_pct.map(|b| format!("{b}%")).unwrap_or("–".into()));
                        stat_row(ui, "Memory", match (g.vram_used_mb, g.vram_total_mb) {
                            (Some(u), Some(t)) => format!("{u} / {t} MB"),
                            _ => "–".into(),
                        });
                        stat_row(ui, "Power", g.power_w.map(|w| format!("{w:.1} W")).unwrap_or("–".into()));
                        stat_row(ui, "Device", format!("{} ({})", g.card, g.device_id));
                    }
                });
            }
        });
        ui.add_space(4.0);
        let eco = if self.snap.cardwire_mode.is_some() {
            "Eco and Standard switch through Cardwire, instantly and both ways."
        } else {
            "Eco switches the discrete GPU off in the firmware; install Cardwire for instant switching back."
        };
        ui.label(
            RichText::new(format!("{eco} Ultimate, and leaving it, take effect after a restart.")).color(DIM()),
        );
    }

    /// Settings › App: start-up, tray, theme, Home layout, sizes, Neon options.
    fn app_settings(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let mut on = self.autostart;
            if switch(ui, &mut on).changed() {
                match tray::set_autostart(on) {
                    Ok(()) => self.autostart = on,
                    Err(e) => self.tray_error = Some(format!("autostart: {e}")),
                }
            }
            ui.label("Start ArcheryCrate when I log in (in the tray)");
        });
        ui.horizontal(|ui| {
            let mut on = self.settings.tray_icon;
            if switch(ui, &mut on).changed() {
                self.settings.tray_icon = on;
                self.settings.save();
                if on && self.shot.is_none() {
                    self.start_tray();
                } else if !on {
                    crate::daemon::quit_tray();
                }
            }
            ui.label("Show an icon in the system tray");
        });
        ui.horizontal(|ui| {
            let mut on = self.settings.close_to_tray;
            if switch(ui, &mut on).changed() {
                self.settings.close_to_tray = on;
                self.settings.save();
            }
            ui.label("Closing the window keeps it running in the tray");
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Theme").color(DIM()));
            let opts = [
                settings::Theme::System,
                settings::Theme::Dark,
                settings::Theme::Light,
                settings::Theme::Neon,
                settings::Theme::NeonLight,
            ];
            let mut i = opts.iter().position(|t| *t == self.settings.theme).unwrap_or(0);
            if segmented(ui, &["Follow system", "Dark", "Light", "Neon", "Light Neon"], &mut i) {
                self.settings.theme = opts[i];
                self.settings.save();
                if opts[i] == settings::Theme::System {
                    // Look the desktop's preference up now rather than in a few seconds.
                    let now = crate::widgets::system_prefers_dark();
                    if let Ok(mut g) = self.system_dark.lock() {
                        *g = now;
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Home layout").color(DIM()));
            let mut i = self.settings.home_grid as usize;
            if segmented(ui, &["Detailed", "3×3 grid"], &mut i) {
                self.settings.home_grid = i == 1;
                self.settings.save();
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Home graphs show").color(DIM()));
            let ranges = [60u32, 300, 900];
            let secs = self.settings.home_range_secs.unwrap_or(60);
            let mut i = ranges.iter().position(|r| *r == secs).unwrap_or(0);
            if segmented(ui, &["1 min", "5 min", "15 min"], &mut i) {
                self.settings.home_range_secs = Some(ranges[i]);
                self.settings.save();
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Interface size").color(DIM()));
            let sizes = [None, Some(0.8f32), Some(0.9), Some(1.0), Some(1.1)];
            let mut i = sizes
                .iter()
                .position(|z| match (z, self.settings.ui_scale) {
                    (None, None) => true,
                    (Some(a), Some(b)) => (a - b).abs() < 0.01,
                    _ => false,
                })
                .unwrap_or(0);
            if segmented(ui, &["Auto", "80%", "90%", "100%", "110%"], &mut i) {
                self.settings.ui_scale = sizes[i];
                self.settings.save();
            }
        });
        ui.label(
            RichText::new("Auto shrinks everything to fit small screens and windows (such as 720p).")
                .color(DIM())
                .size(12.5),
        );
        // The theme on screen (a --theme override for screenshots counts too).
        let neon = matches!(self.theme_override.unwrap_or(self.settings.theme), settings::Theme::Neon | settings::Theme::NeonLight);
        // Glowing lines and the moving rainbow on the graphs, for this kind of theme: Dark and
        // Light keep one choice (off at first), the two Neon themes another (on at first).
        ui.horizontal(|ui| {
            let mut on = if neon { !self.settings.neon_plain_graphs } else { self.settings.bold_graphs };
            if switch(ui, &mut on).changed() {
                if neon {
                    self.settings.neon_plain_graphs = !on;
                } else {
                    self.settings.bold_graphs = on;
                }
                self.settings.save();
            }
            ui.label("Bold rainbow graphs");
            ui.label(RichText::new("(glowing lines and a moving rainbow)").color(DIM()));
        });
        if neon || self.settings.bold_graphs {
            ui.horizontal(|ui| {
                let mut calm = self.settings.neon_calm;
                if switch(ui, &mut calm).changed() {
                    self.settings.neon_calm = calm;
                    self.settings.save();
                }
                ui.label("Calm it down");
                ui.label(RichText::new("(the rainbow stops moving)").color(DIM()));
            });
            ui.add_enabled_ui(!self.settings.neon_calm, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Rainbow speed").color(DIM()));
                    let mut sp = self.settings.rainbow_speed.unwrap_or(1.0);
                    ui.spacing_mut().slider_width = 220.0;
                    let r = ui.add(
                        egui::Slider::new(&mut sp, 0.25..=3.0)
                            .step_by(0.25)
                            .custom_formatter(|v, _| format!("{v:.2}×")),
                    );
                    if r.changed() {
                        self.settings.rainbow_speed = Some(sp);
                        crate::widgets::set_rainbow_speed(sp);
                    }
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.settings.save();
                    }
                    ui.label(RichText::new(match sp {
                        s if s < 0.75 => "slow",
                        s if s <= 1.25 => "normal",
                        _ => "fast",
                    }).color(DIM()));
                });
            });
        }
        ui.horizontal(|ui| {
            let can = self.settings.tray_icon && self.tray_error.is_none();
            ui.add_enabled_ui(can, |ui| {
                if ui.button("Minimize to tray now").clicked() {
                    self.close_to_tray();
                }
            });
            if ui.button("Quit ArcheryCrate").clicked() {
                self.quit();
            }
        });
        if let Some(e) = &self.tray_error {
            ui.label(RichText::new(format!("Tray unavailable: {e}")).color(AMBER()));
        }
    }

    /// Settings › Display and firmware: panel overdrive, start-up sound, the screen's mode.
    fn firmware_settings(&mut self, ui: &mut Ui) {
        let mut pending = Vec::new();
        for name in ["panel_overdrive", "boot_sound"] {
            if let Some(a) = self.snap.attr(name) {
                let mut on = a.current.unwrap_or(0) != 0;
                ui.horizontal(|ui| {
                    if switch(ui, &mut on).changed() {
                        pending.push(ctl::set_attr(name, &a.path, on as i64));
                    }
                    ui.label(match name {
                        "panel_overdrive" => "Panel overdrive (sharper motion on the screen)",
                        _ => "Play the start-up sound",
                    });
                });
            }
        }
        if self.snap.attrs.is_empty() {
            ui.label(RichText::new("No firmware settings found on this kernel.").color(DIM()));
        }
        for p in pending {
            self.run(p);
        }
        let info = self.panel_info.lock().ok().and_then(|g| g.clone());
        if let Some(i) = info {
            ui.label(RichText::new(format!("Screen now: {i}")).color(DIM()).size(12.5));
        }
    }

    fn system(&mut self, ui: &mut Ui) {
        page_title(ui, "Settings", "Setup, how ArcheryCrate starts, plus display and firmware options.");
        self.setup_card(ui);
        // Side by side when there's room for the App card's rows of buttons; stacked otherwise,
        // so the two never overlap in a narrow window.
        if ui.available_width() >= 1250.0 {
            ui.columns(2, |cols| {
                card(&mut cols[0], "App", |ui| self.app_settings(ui));
                card(&mut cols[1], "Display and firmware", |ui| self.firmware_settings(ui));
            });
        } else {
            card(ui, "App", |ui| self.app_settings(ui));
            ui.add_space(4.0);
            card(ui, "Display and firmware", |ui| self.firmware_settings(ui));
        }
        ui.add_space(4.0);
        card(ui, "About ArcheryCrate", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("ArcheryCrate").size(18.0).color(TEXT()));
                ui.label(
                    RichText::new(format!("version {}", env!("CARGO_PKG_VERSION")))
                        .size(18.0)
                        .strong()
                        .color(CYAN()),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("New versions and release notes:").color(DIM()));
                ui.hyperlink_to("github.com/redbeard1083/ArcheryCrate/releases", RELEASES_URL);
            });
        });
        ui.add_space(4.0);
        card(ui, "About this laptop", |ui| {
            let s = &self.snap;
            let t = &self.tools;
            ui.columns(2, |c| {
                stat_row(&mut c[0], "Model", short_model(&s.model));
                stat_row(&mut c[0], "Processor", s.cpu_name.clone());
                stat_row(&mut c[0], "Kernel", s.kernel.clone());
                stat_row(&mut c[0], "Memory", s.mem_total_gb.map(|m| format!("{m:.0} GB")).unwrap_or("–".into()));
                stat_row(&mut c[1], "asusctl", t.asusctl.clone().unwrap_or_else(|| "not installed".into()));
                stat_row(&mut c[1], "asusctl service", if t.asusd_active { "running".into() } else { "stopped".into() });
                stat_row(&mut c[1], "Password helper", if t.pkexec { "pkexec".into() } else { "not found".into() });
                stat_row(
                    &mut c[1],
                    "Running as",
                    if t.flatpak { "Flatpak".into() } else if t.appimage { "AppImage".into() } else { "native program".into() },
                );
            });
        });
    }

    fn activity(&mut self, ui: &mut Ui) {
        page_title(ui, "Activity", "Every change ArcheryCrate made and what the system answered.");
        let background = crate::activity::recent();
        if !background.is_empty() {
            egui::CollapsingHeader::new(
                RichText::new(format!("Done by the tray icon ({}) — charger plugged in or out, tray menu", background.len()))
                    .color(TEXT()),
            )
            .default_open(self.log.is_empty())
            .show(ui, |ui| {
                for (secs, o) in &background {
                    card(ui, "", |ui| {
                        ui.horizontal(|ui| {
                            let (c, m) = if o.ok { (GREEN(), "✔") } else { (RED(), "✖") };
                            ui.label(RichText::new(m).color(c));
                            ui.label(RichText::new(&o.label).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(ago(Duration::from_secs(*secs))).color(DIM()));
                            });
                        });
                        for l in &o.log {
                            ui.label(RichText::new(l).monospace().size(12.0).color(DIM()));
                        }
                    });
                }
            });
            ui.add_space(6.0);
        }
        if self.log.is_empty() {
            card(ui, "", |ui| {
                ui.label(RichText::new("Nothing changed yet.").color(DIM()));
            });
        }
        for (at, o) in self.log.iter().rev() {
            card(ui, "", |ui| {
                ui.horizontal(|ui| {
                    let (c, m) = if o.ok { (GREEN(), "✔") } else { (RED(), "✖") };
                    ui.label(RichText::new(m).color(c));
                    ui.label(RichText::new(&o.label).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(ago(at.elapsed())).color(DIM()));
                    });
                });
                for l in &o.log {
                    ui.label(RichText::new(l).monospace().size(12.0).color(DIM()));
                }
            });
        }
    }

    /// Send the newest lighting effect once edits pause, one command at a time, so a
    /// colour-picker drag doesn't queue dozens of asusctl calls.
    fn send_pending_aura(&mut self) {
        let Some(at) = self.aura_pending else { return };
        let wait = Duration::from_millis(250);
        if self.aura_running || at.elapsed() < wait {
            self.ctx.request_repaint_after(wait);
            return;
        }
        self.aura_pending = None;
        self.aura_running = true;
        self.settings.save();
        let aura = self.settings.aura.clone();
        self.run(ctl::set_aura(&aura));
    }

    fn recheck_tools(&mut self) {
        self.tools.checked = false;
        self.last_tool_check = Instant::now();
        let c = self.ctx.clone();
        ctl::detect_tools(self.tx.clone(), move || c.request_repaint());
    }

    /// While something is missing or stopped, look again every few seconds in the
    /// background, so an install or service start done outside the app shows up by itself.
    fn auto_recheck_tools(&mut self) {
        let incomplete = self.tools.asusctl.is_none() || !self.tools.asusd_active;
        if self.tools.checked && incomplete && self.last_tool_check.elapsed() > Duration::from_secs(5) {
            self.last_tool_check = Instant::now();
            let c = self.ctx.clone();
            ctl::detect_tools(self.tx.clone(), move || c.request_repaint());
        }
    }

    fn setup_card(&mut self, ui: &mut Ui) {
        let t = self.tools.clone();
        let fam = self.distro.family;
        card(ui, "asusctl setup", |ui| {
            ui.label(
                RichText::new(
                    "ArcheryCrate is the control panel; asusctl is the service that talks to the laptop. \
                     It is installed once on the system from your Linux's packages.",
                )
                .color(DIM()),
            );
            ui.add_space(4.0);
            let row = |ui: &mut Ui, ok: Option<bool>, what: &str, detail: String| {
                ui.horizontal(|ui| {
                    let (m, c) = match ok {
                        None => ("…", DIM()),
                        Some(true) => ("✔", GREEN()),
                        Some(false) => ("✖", RED()),
                    };
                    ui.label(RichText::new(m).color(c).strong());
                    ui.label(RichText::new(what).color(TEXT()));
                    ui.label(RichText::new(detail).color(DIM()));
                });
            };
            let known = t.checked;
            let k = |b: bool| if known { Some(b) } else { None };
            row(ui, k(t.asusctl.is_some()), "asusctl installed", t.asusctl.clone().unwrap_or_default());
            row(ui, k(t.asusd_active), "asusctl service (asusd) running", String::new());
            // Cardwire: switches Eco ↔ Standard instantly, both ways.
            let mut cardwire_click = None;
            ui.horizontal(|ui| {
                let (m, c, detail) = match (known, t.cardwire) {
                    (false, _) => ("…", DIM(), ""),
                    (true, Some(true)) => ("✔", GREEN(), "switches between Eco and Standard instantly"),
                    (true, Some(false)) => ("✖", RED(), "installed, but its service isn't running"),
                    (true, None) => ("✖", AMBER(), "not installed: Eco can't switch back without a restart"),
                };
                ui.label(RichText::new(m).color(c).strong());
                ui.label(RichText::new("Cardwire").color(TEXT()));
                ui.label(RichText::new(detail).color(DIM()));
                if known && t.cardwire == Some(false) && t.pkexec && ui.small_button("Start it").clicked() {
                    cardwire_click = Some(Action {
                        label: "Start the Cardwire service".into(),
                        groups: vec![vec![vec![
                            "pkexec".into(),
                            "systemctl".into(),
                            "enable".into(),
                            "--now".into(),
                            "cardwired".into(),
                        ]]],
                        ..Default::default()
                    });
                }
                if known && t.cardwire.is_none() {
                    match &t.cardwire_install {
                        Some(how @ setup::CardwireInstall::Repo) if t.pkexec => {
                            if ui.small_button("Install Cardwire").on_hover_text("Asks for your password.").clicked() {
                                cardwire_click = setup::cardwire_action(how);
                            }
                        }
                        Some(how @ setup::CardwireInstall::Aur(helper)) => {
                            if ui
                                .small_button("Install Cardwire")
                                .on_hover_text(format!("Opens a terminal running {helper} (from the AUR); it asks for your password there."))
                                .clicked()
                            {
                                cardwire_click = setup::cardwire_action(how);
                            }
                        }
                        _ => {
                            ui.hyperlink_to("How to install", setup::CARDWIRE_URL);
                        }
                    }
                }
            });
            if let Some(a) = cardwire_click {
                self.run(a);
            }
            row(ui, k(t.pkexec), "password prompt helper (pkexec)", String::new());
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Your system: {}", self.distro.name)).color(DIM()));
            let cmd = setup::manual_command(fam, t.cardwire_install.as_ref());
            egui::Frame::new()
                .fill(CODE_BG())
                .corner_radius(6)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(&cmd).monospace().size(12.5).color(TEXT()));
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let missing = known && t.asusctl.is_none();
                let stopped = known && t.asusctl.is_some() && !t.asusd_active;
                let can_install = setup::install_script(fam).is_some() && t.pkexec;
                if missing {
                    ui.add_enabled_ui(can_install && self.busy == 0, |ui| {
                        if accent_button(ui, "Install for me")
                            .on_hover_text(
                                "Asks for your password, installs asusctl and starts its service \
                                 (plus Cardwire, when your Linux's repositories have it).",
                            )
                            .clicked()
                        {
                            if let Some(a) = setup::install_action(fam) {
                                self.run(a);
                            }
                        }
                    });
                } else if stopped && t.pkexec {
                    if accent_button(ui, "Start the service").clicked() {
                        self.run(setup::start_service_action());
                    }
                }
                if ui.button("Copy command").clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
                if ui.button("Check again").clicked() {
                    self.recheck_tools();
                }
                if self.busy > 0 {
                    ui.spinner();
                }
            });
            if known && t.asusctl.is_none() && setup::install_script(fam).is_none() {
                ui.label(RichText::new("Automatic install isn't available for this Linux; follow the guide above.").color(AMBER()));
            }
        });
    }
}

/// "1 second", "90 seconds".
fn seconds(n: usize) -> String {
    match n.max(1) {
        1 => "1 second".into(),
        n => format!("{n} seconds"),
    }
}

/// The palette for a theme choice (`system`: the desktop prefers dark, when known).
fn palette_for(theme: settings::Theme, system: Option<bool>) -> crate::widgets::Palette {
    use crate::widgets::Palette;
    match theme {
        settings::Theme::Dark => Palette::Dark,
        settings::Theme::Light => Palette::Light,
        settings::Theme::Neon => Palette::Neon,
        settings::Theme::NeonLight => Palette::NeonLight,
        settings::Theme::System => {
            if system.unwrap_or(true) {
                Palette::Dark
            } else {
                Palette::Light
            }
        }
    }
}

/// The GPU the Home page follows: the discrete one while it's awake (in use), otherwise the
/// integrated one that drives the screen.
fn home_gpu(s: &Snapshot) -> Option<crate::hw::Gpu> {
    match s.dgpu() {
        Some(d) if d.awake => Some(d.clone()),
        _ => s.igpu().cloned().or_else(|| s.dgpu().cloned()),
    }
}

fn ago(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else {
        format!("{} h ago", s / 3600)
    }
}

/// The title and text of the "this needs a restart" question for switching to `m`
/// (the window's dialog and the tray's notification).
pub fn gfx_restart_text(m: GfxMode) -> (&'static str, &'static str) {
    if m == GfxMode::Ultimate {
        (
            "Switch to Ultimate?",
            "Ultimate wires the screen straight to the discrete GPU. The laptop has to restart \
             before it takes effect, and the discrete GPU stays on until you switch back \
             (which also needs a restart).",
        )
    } else if m == GfxMode::Eco {
        (
            "Leave Ultimate first",
            "The discrete GPU is driving the screen, so it can't be switched off yet. \
             ArcheryCrate hands the screen back to the integrated GPU now (Standard after the \
             restart); then pick Eco again to switch the discrete GPU off.",
        )
    } else {
        (
            "Leave Ultimate?",
            "Handing the screen back to the integrated GPU only takes effect after the laptop restarts.",
        )
    }
}

/// One line of a combined graph and the column above it: a name with a colour dot, the value
/// now in large type (or a note when idle), a few detail lines, and the history.
struct Col {
    name: String,
    color: Color32,
    value: Option<String>,
    note: String,
    lines: Vec<String>,
    dim: bool,
    hist: Vec<f32>,
}

/// Columns side by side over one graph holding all their lines (Home's Load, Fans, Memory).
fn combined_graph(ui: &mut Ui, cols: &[Col], range: (f32, f32), ticks: &[f32], unit: &str, height: f32) {
    ui.columns(cols.len().max(1), |c| {
        for (k, col) in cols.iter().enumerate() {
            let ui = &mut c[k];
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, if col.dim { DIM() } else { col.color });
                ui.label(RichText::new(&col.name).color(TEXT()));
            });
            match &col.value {
                Some(v) => ui.label(RichText::new(v).size(34.0).strong().color(col.color)),
                None => ui.label(RichText::new(&col.note).size(26.0).color(DIM())),
            };
            for l in col.lines.iter().filter(|l| !l.is_empty()) {
                ui.label(RichText::new(l).color(DIM()).size(12.5));
            }
        }
    });
    ui.add_space(6.0);
    let series: Vec<crate::widgets::Series> =
        cols.iter().map(|c| crate::widgets::Series { values: &c.hist, color: c.color, dim: c.dim }).collect();
    multi_graph(ui, &series, range, ticks, unit, height);
}

/// The most points a Home graph draws; longer histories are averaged down to this.
const GRAPH_POINTS: usize = 240;

/// The last `secs` readings of a history (about one a second), averaged down to at most
/// `GRAPH_POINTS` for drawing.
fn recent(q: &VecDeque<f32>, secs: usize) -> Vec<f32> {
    let n = q.len().min(secs);
    let v: Vec<f32> = q.iter().skip(q.len() - n).copied().collect();
    if v.len() <= GRAPH_POINTS {
        return v;
    }
    let k = v.len().div_ceil(GRAPH_POINTS);
    v.chunks(k).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect()
}

/// The highest reading in the last `secs` (before averaging).
fn recent_peak(q: &VecDeque<f32>, secs: usize) -> Option<f32> {
    q.iter().rev().take(secs).copied().reduce(f32::max)
}

/// "last 45 seconds", "last 5 minutes": how much of the range the history covers.
fn span_text(len: usize, secs: usize) -> String {
    let n = len.min(secs);
    if n < 120 { format!("last {}", seconds(n)) } else { format!("last {} minutes", n / 60) }
}

/// "3 h 20 min", "45 min".
fn duration_text(hours: f32) -> String {
    let mut m = (hours * 60.0).round().max(1.0) as u32;
    if m >= 60 {
        m = (m + 2) / 5 * 5; // an estimate: whole five minutes
        format!("{} h {} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// "About 3 h 20 min left at 14 W" while draining, "About 45 min to 80% at 30 W" while
/// charging, from the battery's energy and its average draw over the last minute.
fn battery_eta(s: &Snapshot, avg_w: Option<f32>) -> Option<String> {
    let w = avg_w.filter(|w| *w > 0.5)?;
    let now = s.battery_energy_wh?;
    match s.battery_status.as_str() {
        "Discharging" => Some(format!("About {} left at {w:.0} W", duration_text(now / w))),
        "Charging" => {
            let limit = s.charge_limit.unwrap_or(100).min(100) as f32;
            let target = s.battery_full_wh? * limit / 100.0;
            (target > now).then(|| format!("About {} to {limit:.0}% at {w:.0} W", duration_text((target - now) / w)))
        }
        _ => None,
    }
}

const RELEASES_URL: &str = "https://github.com/redbeard1083/ArcheryCrate/releases";

fn temp_color(t: f32) -> Color32 {
    heat_color(t)
}

/// Battery: green, amber when low, red when nearly empty.
fn battery_color(b: f32) -> Color32 {
    if b < 15.0 {
        RED()
    } else if b < 30.0 {
        AMBER()
    } else {
        GREEN()
    }
}

fn fan_label(l: &str) -> String {
    match l {
        "cpu_fan" => "CPU fan".into(),
        "gpu_fan" => "GPU fan".into(),
        "mid_fan" => "Middle fan".into(),
        o => o.to_string(),
    }
}

fn short_model(m: &str) -> String {
    // "ROG Zephyrus G14 GA402RJ_GA402RJ" → drop the repeated board name.
    match m.rsplit_once('_') {
        Some((a, b)) if a.ends_with(b) => a.to_string(),
        _ => m.to_string(),
    }
}

/// The window size the pages are laid out for; smaller windows are scaled down to it.
const DESIGN_W: f32 = 1120.0;
const DESIGN_H: f32 = 780.0;

impl CrateApp {
    /// Small screens (a 720p laptop): start maximised, and scale the interface down so the
    /// pages still fit; Settings › Interface size can fix the size instead.
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() && !self.screen_fitted {
            if let Some(m) = ctx.input(|i| i.viewport().monitor_size) {
                self.screen_fitted = true;
                if m.x < 1400.0 || m.y < 900.0 {
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
                }
            }
        }
        let zoom = ctx.zoom_factor();
        // The window's size before any scaling, so the choice doesn't feed back on itself.
        let native = ctx.content_rect().size() * zoom;
        let fit = (native.x / DESIGN_W).min(native.y / DESIGN_H).clamp(0.7, 1.0);
        let want = self.settings.ui_scale.unwrap_or(fit);
        if (want - zoom).abs() > 0.01 {
            ctx.set_zoom_factor(want);
        }
    }
}

impl eframe::App for CrateApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.auto_recheck_tools();
        self.send_pending_aura();
        self.take_stock();
        self.send_pending_fans();
        self.follow_theme(ctx);
        self.fit_to_screen(ctx);
        // The window really closes (Wayland can't hide one); whether the tray icon stays
        // running in its own process decides between "close to tray" and "quit".
        if ctx.input(|i| i.viewport().close_requested()) && self.shot.is_none() {
            if !self.quitting && self.settings.close_to_tray && self.settings.tray_icon {
                self.start_tray();
            } else {
                crate::daemon::quit_tray();
            }
            let _ = std::fs::remove_file(crate::daemon::window_socket());
        }
        if let Some(shot) = &self.shot {
            if shot.done.load(Ordering::SeqCst) {
                if let Some(e) = shot.error.lock().ok().and_then(|g| g.clone()) {
                    eprintln!("screenshot failed: {e}");
                    std::process::exit(1);
                }
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(1000));
        if self.shot.is_some() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if crate::widgets::is_neon() {
            // Neon: a thin, moving rainbow along the top of the window.
            egui::Panel::top("rainbow")
                .resizable(false)
                .exact_size(3.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(BG()))
                .show(ui, |ui| {
                    let r = ui.max_rect();
                    crate::widgets::paint_rainbow(ui.painter(), r);
                });
        }
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(210.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDEBAR()))
            .show(ui, |ui| self.sidebar(ui));
        egui::Panel::bottom("status")
            .resizable(false)
            .exact_size(40.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDEBAR()).inner_margin(egui::Margin::symmetric(8, 6)))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG()).inner_margin(egui::Margin::symmetric(24, 18)))
            .show(ui, |ui| {
                // The page's visible height (the 3×3 Home grid fits itself into it).
                self.view_h = ui.available_height();
                let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                // Screenshot checks can photograph the bottom of a page (ARCHERYCRATE_SCROLL_END=1).
                if self.shot.is_some() && std::env::var_os("ARCHERYCRATE_SCROLL_END").is_some() {
                    area = area.stick_to_bottom(true);
                }
                area.show(ui, |ui| {
                    self.page_ui(ui);
                    ui.add_space(12.0);
                });
            });
        let ctx = ui.ctx().clone();
        self.dialogs(&ctx);

        if let Some(shot) = &mut self.shot {
            shot.frames += 1;
            if shot.frames >= 120 && !shot.requested {
                shot.requested = true;
                let path = shot.path.clone();
                let done = shot.done.clone();
                let err = shot.error.clone();
                let c = ui.ctx().clone();
                ui.ctx().request_screenshot(move |img| {
                    let [w, h] = img.size;
                    let mut bytes = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        bytes.extend_from_slice(&p.to_srgba_unmultiplied());
                    }
                    let res = (|| -> Result<(), String> {
                        if let Some(d) = path.parent() {
                            if !d.as_os_str().is_empty() {
                                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                            }
                        }
                        let im = image::RgbaImage::from_raw(w as u32, h as u32, bytes).ok_or("bad image size")?;
                        im.save(&path).map_err(|e| e.to_string())
                    })();
                    if let Err(e) = res {
                        *err.lock().unwrap() = Some(e);
                    }
                    done.store(true, Ordering::SeqCst);
                    c.request_repaint();
                });
            }
        }
    }
}
