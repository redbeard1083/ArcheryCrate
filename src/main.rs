//! ArcheryCrate — a control center for ASUS laptops on Linux, built on asusctl.

mod activity;
mod migrate;
mod app;
mod ctl;
mod daemon;
mod display;
mod power;
mod hw;
mod settings;
mod setup;
mod tray;
mod widgets;

use std::path::PathBuf;

use eframe::egui;

/// Draw the window on the integrated GPU (the discrete one in Ultimate mode).
///
/// egui prefers the "high performance" GPU by default, which on a hybrid laptop is the
/// discrete one. Switching to Eco mode then pulls that GPU out from under the window and
/// the renderer aborts; it also keeps the discrete GPU awake just to draw a control panel.
fn window_gpu() -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu::{Adapter, DeviceType, PowerPreference};

    // In Ultimate mode the discrete GPU drives the screen and the integrated one has nothing
    // to show: drawing there would keep it busy and copy every frame across. Eco can't be
    // chosen in Ultimate without a restart, so the window can safely use the discrete GPU then.
    let ultimate = hw::dgpu_drives_builtin_screen();
    let mut create = WgpuSetupCreateNew::without_display_handle();
    create.power_preference = if ultimate.is_some() { PowerPreference::HighPerformance } else { PowerPreference::LowPower };
    create.native_adapter_selector = Some(std::sync::Arc::new(move |adapters: &[Adapter], surface| {
        let usable: Vec<&Adapter> = adapters
            .iter()
            .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
            .collect();
        let rank = |a: &Adapter| {
            let info = a.get_info();
            match (ultimate, info.device_type) {
                (Some((v, d)), _) if info.vendor == v && info.device == d => 0,
                (Some(_), DeviceType::DiscreteGpu) => 1,
                (Some(_), _) => 4,
                (None, DeviceType::IntegratedGpu) => 0,
                (None, DeviceType::Other | DeviceType::VirtualGpu) => 1,
                (None, DeviceType::Cpu) => 2,
                (None, DeviceType::DiscreteGpu) => 3,
            }
        };
        usable
            .into_iter()
            .min_by_key(|a| rank(a))
            .cloned()
            .ok_or_else(|| "no graphics adapter can draw to this window".to_string())
    }));
    WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(create),
        // Don't wait for vsync: on Wayland that means waiting for the compositor's "draw the
        // next frame" signal, which can stop coming while monitors are added or removed (a
        // monitor plugged in with the charger froze the window). The window only redraws
        // when something changes, so this doesn't make it busy.
        surface: eframe::egui_wgpu::SurfaceConfig {
            present_mode: eframe::wgpu::PresentMode::AutoNoVsync,
            desired_maximum_frame_latency: Some(2),
        },
        ..Default::default()
    }
}

fn main() -> eframe::Result<()> {
    let mut screenshot: Option<PathBuf> = None;
    let mut hidden = false;
    let mut page: Option<String> = None;
    let mut gfx: Option<ctl::GfxMode> = None;
    let mut theme: Option<settings::Theme> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--screenshot" => screenshot = args.next().map(PathBuf::from),
            "--hidden" | "--minimized" | "--tray" => hidden = true,
            "--page" => page = args.next(),
            // Opened from the tray's graphics menu: switch, or ask first when a restart is needed.
            "--gfx" => gfx = args.next().as_deref().and_then(ctl::GfxMode::from_key),
            // For checks and screenshots: this run only, the saved choice is left alone.
            "--theme" => {
                theme = match args.next().as_deref() {
                    Some("light") => Some(settings::Theme::Light),
                    Some("dark") => Some(settings::Theme::Dark),
                    Some("system") => Some(settings::Theme::System),
                    Some("neon") => Some(settings::Theme::Neon),
                    _ => None,
                }
            }
            "--help" | "-h" => {
                println!(
                    "ArcheryCrate {}\n\nUsage: archerycrate [--hidden] [--page <name>] [--screenshot <file.png>]\n\n  --hidden       start in the system tray\n  --screenshot   draw the window, save it as a PNG and exit",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "--version" | "-V" => {
                println!("ArcheryCrate {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }
    }
    if screenshot.is_some() {
        hidden = false;
    } else {
        // Carry settings etc. over from the old name (ArmoryCraft), once.
        migrate::run();
        // Gives the taskbar the right icon on Wayland (only when run as an AppImage).
        tray::integrate_appimage();
        if hidden {
            // "Start in the tray": just the tray icon; it opens a window when clicked.
            daemon::run_tray();
            return Ok(());
        }
        // Already open? Bring that window up instead of opening a second one.
        let msg = gfx.map(|m| format!("gfx {}", m.key())).unwrap_or_else(|| "show".into());
        if daemon::send(&daemon::window_socket(), &msg) {
            return Ok(());
        }
    }

    let icon = egui::IconData { rgba: tray::icon_rgba(64), width: 64, height: 64 };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("ArcheryCrate")
        .with_app_id("io.github.archerycrate.ArcheryCrate")
        .with_inner_size([1240.0, 820.0])
        // Small enough for a 720p screen; the pages scale down to fit (see `fit_to_screen`).
        .with_min_inner_size([760.0, 480.0])
        .with_icon(icon)
        .with_visible(!hidden);
    if screenshot.is_some() {
        // Never steal focus from the person while a check runs.
        viewport = viewport.with_active(false);
        // ARCHERYCRATE_SHOT_SIZE=1280x660 photographs the window at another size (e.g. 720p).
        if let Some((w, h)) = std::env::var("ARCHERYCRATE_SHOT_SIZE")
            .ok()
            .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))))
        {
            viewport = viewport.with_inner_size([w, h]);
        }
    }

    let options = eframe::NativeOptions { viewport, wgpu_options: window_gpu(), ..Default::default() };
    eframe::run_native(
        "ArcheryCrate",
        options,
        Box::new(move |cc| Ok(Box::new(app::CrateApp::new(cc, screenshot, page, gfx, theme)))),
    )
}
