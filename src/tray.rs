//! System tray icon (StatusNotifierItem) and "start when I log in".

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use ksni::blocking::TrayMethods;

pub enum TrayCmd {
    /// Open or raise the window, with the desktop's activation token when it gave one
    /// (Wayland only lets a window take focus with such a token).
    Show(Option<String>),
    Profile(String),
    /// A graphics mode picked in the tray menu (with an activation token, as for Show).
    Gfx(crate::ctl::GfxMode, Option<String>),
    Quit,
}

pub struct Tray {
    tx: Sender<TrayCmd>,
    wake: Box<dyn Fn() + Send>,
    pub profile: String,
    pub choices: Vec<String>,
    pub status: String,
    /// The graphics mode now, or None when this laptop has no graphics switches.
    pub gfx: Option<crate::ctl::GfxMode>,
    /// The graphics mode chosen for after the next restart.
    pub gfx_pending: Option<crate::ctl::GfxMode>,
    /// Briefly true to make the panel redraw the graphics choices: it ticks a clicked choice
    /// by itself, which must not stay ticked when nothing changed (or not yet).
    pub gfx_blank: bool,
}

/// A greyed-out line used as a heading inside the menu.
fn heading<T: ksni::Tray>(label: &str) -> ksni::MenuItem<T> {
    ksni::menu::StandardItem { label: label.into(), enabled: false, ..Default::default() }.into()
}

impl Tray {
    fn send(&self, c: TrayCmd) {
        let _ = self.tx.send(c);
        (self.wake)();
    }
}

pub fn icon_rgba(size: u32) -> Vec<u8> {
    // A red ring with a bright core, drawn here so no artwork has to ship.
    // Same shapes as packaging/io.github.archerycrate.ArcheryCrate.svg, with soft edges.
    let mut px = vec![0u8; (size * size * 4) as usize];
    let c = size as f32 / 2.0;
    // How much of a 1-pixel-wide edge at radius `edge` (in pixels) lies inside.
    let inside = |d: f32, edge: f32| (edge - d + 0.5).clamp(0.0, 1.0);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let d = (dx * dx + dy * dy).sqrt();
            // Layers from the back: halo, core, ring ("over" blending, straight alpha).
            let layers = [
                ([255.0, 120.0, 150.0], 0.78 * inside(d, 0.46 * c)),
                ([255.0, 255.0, 255.0], inside(d, 0.38 * c)),
                ([255.0, 40.0, 80.0], inside(d, 0.92 * c) * (1.0 - inside(d, 0.62 * c))),
            ];
            let (mut rgb, mut a) = ([0.0f32; 3], 0.0f32);
            for (col, la) in layers {
                let na = la + a * (1.0 - la);
                if na > 0.0 {
                    for k in 0..3 {
                        rgb[k] = (col[k] * la + rgb[k] * a * (1.0 - la)) / na;
                    }
                }
                a = na;
            }
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&[rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, (a * 255.0).round() as u8]);
        }
    }
    px
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "archerycrate".into()
    }
    fn title(&self) -> String {
        "ArcheryCrate".into()
    }
    fn icon_name(&self) -> String {
        String::new()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        [22u32, 32, 48]
            .iter()
            .map(|&s| {
                let rgba = icon_rgba(s);
                let mut argb = Vec::with_capacity(rgba.len());
                for p in rgba.chunks(4) {
                    argb.extend_from_slice(&[p[3], p[0], p[1], p[2]]);
                }
                ksni::Icon { width: s as i32, height: s as i32, data: argb }
            })
            .collect()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "ArcheryCrate".into(),
            description: self.status.clone(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCmd::Show(ksni::current_xdg_activation_token()));
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let choices = self.choices.clone();
        let selected = choices.iter().position(|c| *c == self.profile).unwrap_or(usize::MAX);
        let mut items: Vec<ksni::MenuItem<Self>> = vec![
            StandardItem {
                label: "Open ArcheryCrate".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCmd::Show(ksni::current_xdg_activation_token()))),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
        ];
        if !choices.is_empty() {
            items.push(heading("Performance"));
            let opts = choices
                .iter()
                .map(|c| RadioItem { label: crate::app::profile_title(c).to_string(), ..Default::default() })
                .collect();
            items.push(
                RadioGroup {
                    selected,
                    select: Box::new(move |t: &mut Self, i| {
                        if let Some(p) = choices.get(i) {
                            t.send(TrayCmd::Profile(p.clone()));
                        }
                    }),
                    options: opts,
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }
        if let Some(now) = self.gfx {
            use crate::ctl::GfxMode;
            items.push(heading("Graphics"));
            let modes = GfxMode::ALL;
            let pending = self.gfx_pending.filter(|p| *p != now);
            // Ticked: what's chosen, i.e. the mode waiting for the restart when there is one.
            let chosen = pending.unwrap_or(now);
            let opts = modes
                .iter()
                .map(|m| {
                    let label = if pending == Some(*m) {
                        format!("{} (after the restart)", m.title())
                    } else if pending.is_some() && *m == now {
                        format!("{} (until the restart)", m.title())
                    } else if *m != now && (*m == GfxMode::Ultimate || now == GfxMode::Ultimate) {
                        format!("{} (needs a restart)", m.title())
                    } else {
                        m.title().to_string()
                    };
                    RadioItem { label, ..Default::default() }
                })
                .collect();
            items.push(
                RadioGroup {
                    selected: if self.gfx_blank {
                        usize::MAX
                    } else {
                        modes.iter().position(|m| *m == chosen).unwrap_or(usize::MAX)
                    },
                    select: Box::new(move |t: &mut Self, i| {
                        if let Some(m) = modes.get(i) {
                            t.send(TrayCmd::Gfx(*m, ksni::current_xdg_activation_token()));
                        }
                    }),
                    options: opts,
                }
                .into(),
            );
            items.push(MenuItem::Separator);
        }
        items.push(
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCmd::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

pub type Handle = ksni::blocking::Handle<Tray>;

pub fn start(tx: Sender<TrayCmd>, wake: impl Fn() + Send + 'static) -> Result<Handle, String> {
    let tray = Tray {
        tx,
        wake: Box::new(wake),
        profile: String::new(),
        choices: Vec::new(),
        status: String::new(),
        gfx: None,
        gfx_pending: None,
        gfx_blank: false,
    };
    tray.disable_dbus_name(crate::ctl::in_flatpak()).spawn().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Desktop integration for the AppImage
// ---------------------------------------------------------------------------

pub const APP_ID: &str = "io.github.archerycrate.ArcheryCrate";
const ICON_SVG: &str = include_str!("../packaging/io.github.archerycrate.ArcheryCrate.svg");

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// Write `text` to `path` only if it differs, so desktops don't rescan for nothing.
/// Returns whether the file changed.
fn write_if_changed(path: &std::path::Path, text: &[u8]) -> std::io::Result<bool> {
    if std::fs::read(path).ok().as_deref() == Some(text) {
        return Ok(false);
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, text).map(|_| true)
}

fn icon_png(size: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(size, size, icon_rgba(size))?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Ask KDE to re-read app entries and icons (it caches "icon not found" until told).
fn refresh_desktop() {
    let quiet = |c: &mut std::process::Command| {
        c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    };
    let mut c = std::process::Command::new("kbuildsycoca6");
    quiet(&mut c);
    let _ = crate::daemon::spawn_reaped(&mut c);
    let mut c = std::process::Command::new("dbus-send");
    c.args(["--session", "--type=signal", "/KIconLoader", "org.kde.KIconLoader.iconChanged", "int32:0"]);
    quiet(&mut c);
    let _ = crate::daemon::spawn_reaped(&mut c);
    let mut c = std::process::Command::new("update-desktop-database");
    quiet(&mut c);
    if let Some(d) = data_home() {
        c.arg(d.join("applications"));
        let _ = crate::daemon::spawn_reaped(&mut c);
    }
}

/// On Wayland the taskbar finds a window's icon through an installed `.desktop` file whose
/// name matches the window's app id; an AppImage installs nothing, so KDE falls back to its
/// generic "W" icon. Register the AppImage (launcher entry + icon) in the user's own data
/// folder, the way AppImage integration tools do, and keep the path current if it moves.
pub fn integrate_appimage() {
    let Some(appimage) = crate::ctl::own_appimage() else { return };
    let Some(data) = data_home() else { return };
    let icon = data.join("icons/hicolor/scalable/apps").join(format!("{APP_ID}.svg"));
    let desktop = data.join("applications").join(format!("{APP_ID}.desktop"));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=ArcheryCrate\nGenericName=Laptop Control Center\n\
         Comment=Performance modes, fan curves, battery charge limit, keyboard lighting and graphics for ASUS laptops\n\
         Exec=\"{}\"\nIcon={APP_ID}\nTerminal=false\nStartupNotify=true\nCategories=System;Settings;HardwareSettings;\n\
         Keywords=asus;asusctl;fan;battery;rgb;performance;rog;\nStartupWMClass={APP_ID}\n\
         X-ArcheryCrate-AppImage=true\n",
        appimage.to_string_lossy()
    );
    let mut changed = false;
    let mut write = |p: &std::path::Path, bytes: &[u8]| match write_if_changed(p, bytes) {
        Ok(c) => changed |= c,
        Err(e) => eprintln!("could not register the AppImage with the desktop ({}): {e}", p.display()),
    };
    write(&icon, ICON_SVG.as_bytes());
    // Fixed sizes too: some panels and caches only look at PNGs.
    for s in [32u32, 48, 64, 128, 256] {
        if let Some(png) = icon_png(s) {
            let p = data.join(format!("icons/hicolor/{s}x{s}/apps")).join(format!("{APP_ID}.png"));
            write(&p, &png);
        }
    }
    write(&desktop, entry.as_bytes());
    if changed {
        refresh_desktop();
    }
}

// ---------------------------------------------------------------------------
// Autostart (XDG autostart entry)
// ---------------------------------------------------------------------------

fn autostart_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|_| !crate::ctl::in_flatpak()) // inside Flatpak XDG_CONFIG_HOME is the sandbox's
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("autostart").join("archerycrate.desktop")
}

pub fn autostart_enabled() -> bool {
    autostart_path().exists()
}

fn exec_line() -> String {
    if crate::ctl::in_flatpak() {
        let id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "io.github.archerycrate.ArcheryCrate".into());
        return format!("flatpak run {id} --hidden");
    }
    if let Some(a) = crate::ctl::own_appimage() {
        return format!("\"{}\" --hidden", a.to_string_lossy());
    }
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "archerycrate".into());
    format!("\"{exe}\" --hidden")
}

pub fn set_autostart(on: bool) -> Result<(), String> {
    let p = autostart_path();
    if on {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=ArcheryCrate\nComment=Laptop performance, fans, battery and lighting\nExec={}\nIcon=io.github.archerycrate.ArcheryCrate\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\n",
            exec_line()
        );
        std::fs::write(&p, text).map_err(|e| e.to_string())
    } else if p.exists() {
        std::fs::remove_file(&p).map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}
