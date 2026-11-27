//! Look and feel: colours, cards, gauges, switches, the fan-curve editor.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, pos2, vec2,
};

use crate::hw::Curve;

// ---------------------------------------------------------------------------
// Colours: one palette per theme. The names stay the same everywhere; which palette they
// read from is switched by `set_palette`.
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// The palettes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Palette {
    Dark = 0,
    Light = 1,
    /// Cyberpunk: near-black purple backgrounds, neon accents, glow and moving rainbow.
    Neon = 2,
    /// Neon on a pale background: the same glow and rainbow, deeper accents for contrast.
    NeonLight = 3,
}

static PALETTE: AtomicU8 = AtomicU8::new(Palette::Dark as u8);
/// "Calm it down" in Neon: the rainbow stops moving (and the window stops redrawing for it).
static CALM: AtomicBool = AtomicBool::new(false);
/// The rainbow's speed (an f32's bits; 1.0 = one full cycle about every 6 s).
static SPEED: AtomicU32 = AtomicU32::new(0x3f80_0000);

pub fn palette() -> Palette {
    match PALETTE.load(Ordering::Relaxed) {
        1 => Palette::Light,
        2 => Palette::Neon,
        3 => Palette::NeonLight,
        _ => Palette::Dark,
    }
}

/// Dark-based palettes (Dark and Neon).
pub fn is_dark() -> bool {
    matches!(palette(), Palette::Dark | Palette::Neon)
}

/// The Neon look (glow, moving rainbow, line icons), dark or light.
pub fn is_neon() -> bool {
    matches!(palette(), Palette::Neon | Palette::NeonLight)
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Pick the colour for the current palette.
fn pick(dark: Color32, light: Color32, neon: Color32, neon_light: Color32) -> Color32 {
    match palette() {
        Palette::Dark => dark,
        Palette::Light => light,
        Palette::Neon => neon,
        Palette::NeonLight => neon_light,
    }
}

#[allow(non_snake_case)]
pub fn BG() -> Color32 {
    pick(rgb(13, 13, 16), rgb(240, 240, 244), rgb(2, 1, 6), rgb(246, 243, 255))
}
#[allow(non_snake_case)]
pub fn SIDEBAR() -> Color32 {
    pick(rgb(19, 19, 24), rgb(228, 228, 235), rgb(5, 3, 11), rgb(236, 230, 252))
}
#[allow(non_snake_case)]
pub fn CARD() -> Color32 {
    pick(rgb(26, 26, 32), rgb(255, 255, 255), rgb(7, 4, 15), rgb(255, 255, 255))
}
#[allow(non_snake_case)]
pub fn CARD_HI() -> Color32 {
    pick(rgb(34, 34, 42), rgb(234, 234, 241), rgb(18, 10, 34), rgb(238, 232, 252))
}
#[allow(non_snake_case)]
pub fn LINE() -> Color32 {
    pick(rgb(46, 46, 56), rgb(212, 212, 222), rgb(70, 30, 110), rgb(196, 176, 236))
}
#[allow(non_snake_case)]
pub fn TEXT() -> Color32 {
    pick(rgb(232, 232, 238), rgb(26, 26, 32), rgb(232, 246, 255), rgb(22, 14, 46))
}
#[allow(non_snake_case)]
pub fn DIM() -> Color32 {
    pick(rgb(140, 140, 155), rgb(98, 98, 112), rgb(160, 150, 205), rgb(98, 86, 140))
}
#[allow(non_snake_case)]
pub fn ACCENT() -> Color32 {
    pick(rgb(255, 40, 80), rgb(222, 22, 62), rgb(255, 20, 170), rgb(226, 0, 146))
}
#[allow(non_snake_case)]
pub fn ACCENT_DIM() -> Color32 {
    pick(rgb(120, 22, 42), rgb(250, 204, 214), rgb(96, 8, 76), rgb(255, 204, 236))
}
// Deeper accents on light backgrounds so they stay readable on white; electric in Neon.
#[allow(non_snake_case)]
pub fn CYAN() -> Color32 {
    pick(rgb(40, 200, 255), rgb(0, 132, 196), rgb(0, 240, 255), rgb(0, 150, 205))
}
#[allow(non_snake_case)]
pub fn GREEN() -> Color32 {
    pick(rgb(60, 220, 130), rgb(16, 146, 76), rgb(80, 255, 60), rgb(0, 160, 64))
}
#[allow(non_snake_case)]
pub fn AMBER() -> Color32 {
    pick(rgb(255, 176, 32), rgb(184, 112, 0), rgb(255, 232, 0), rgb(204, 132, 0))
}
/// Something is wrong or too hot (red in every theme, never the theme's accent).
#[allow(non_snake_case)]
pub fn RED() -> Color32 {
    pick(rgb(255, 64, 64), rgb(204, 20, 20), rgb(255, 36, 36), rgb(218, 18, 40))
}
#[allow(non_snake_case)]
pub fn ORANGE() -> Color32 {
    pick(rgb(255, 128, 40), rgb(206, 84, 0), rgb(255, 120, 0), rgb(226, 96, 0))
}

/// A temperature's colour on one scale everywhere: cyan when cool, then green, yellow,
/// orange and red as it climbs (smoothly in between).
pub fn heat_color(t: f32) -> Color32 {
    let stops = [(40.0, CYAN()), (55.0, GREEN()), (70.0, AMBER()), (80.0, ORANGE()), (90.0, RED())];
    if t <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if t <= b {
            return lerp_color(ca, cb, (t - a) / (b - a));
        }
    }
    stops[stops.len() - 1].1
}

/// A level that's fine until it gets high (memory, storage): `base`, then amber, then red.
pub fn level_color(frac: f32, base: Color32) -> Color32 {
    if frac >= 0.92 {
        RED()
    } else if frac >= 0.80 {
        AMBER()
    } else {
        base
    }
}

/// Background inside graphs and the fan-curve editor.
#[allow(non_snake_case)]
pub fn GRAPH_BG() -> Color32 {
    pick(rgb(18, 18, 22), rgb(246, 246, 250), rgb(1, 0, 4), rgb(248, 246, 255))
}
#[allow(non_snake_case)]
pub fn GRID() -> Color32 {
    pick(rgb(32, 32, 40), rgb(226, 226, 234), rgb(28, 14, 48), rgb(226, 218, 244))
}
/// Background of command boxes (monospace text).
#[allow(non_snake_case)]
pub fn CODE_BG() -> Color32 {
    pick(rgb(12, 12, 15), rgb(232, 232, 239), rgb(1, 0, 4), rgb(240, 235, 252))
}
#[allow(non_snake_case)]
fn HOVER() -> Color32 {
    pick(rgb(48, 48, 60), rgb(222, 222, 232), rgb(32, 16, 60), rgb(230, 222, 250))
}

/// Which palette to use; returns true when it changed (then call `apply_theme`).
pub fn set_palette(p: Palette) -> bool {
    PALETTE.swap(p as u8, Ordering::Relaxed) != p as u8
}

/// How fast the rainbow moves: 1.0 is normal, 0.25 slow, 3.0 fast.
pub fn set_rainbow_speed(speed: f32) {
    SPEED.store(speed.clamp(0.1, 5.0).to_bits(), Ordering::Relaxed);
}

pub fn set_calm(calm: bool) {
    CALM.store(calm, Ordering::Relaxed);
}

/// Whether Neon's colours are moving right now (the window then keeps redrawing).
pub fn rainbow_moving() -> bool {
    is_neon() && !CALM.load(Ordering::Relaxed)
}

/// Where the moving rainbow is, in turns (0..1 is one full cycle, about every 6 s).
pub fn rainbow_phase() -> f32 {
    // Advanced by the time since the last call at the current speed, so changing the speed
    // doesn't make the colours jump.
    static STATE: Mutex<Option<(Instant, f32)>> = Mutex::new(None);
    if !rainbow_moving() {
        return 0.0;
    }
    let speed = f32::from_bits(SPEED.load(Ordering::Relaxed));
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let (last, phase) = g.unwrap_or((now, 0.0));
    let phase = (phase + now.duration_since(last).as_secs_f32() * speed / 6.0).fract();
    *g = Some((now, phase));
    phase
}

/// A rainbow colour `offset` turns along from the moving phase.
pub fn rainbow(offset: f32) -> Color32 {
    // A little deeper on Light Neon's pale background, so yellow and cyan stay visible.
    if palette() == Palette::NeonLight {
        hsv(rainbow_phase() + offset, 1.0, 0.85)
    } else {
        hsv(rainbow_phase() + offset, 0.95, 1.0)
    }
}

/// Neon glow: a few widening, fading strokes around `rect` (only in Neon).
pub fn glow_rect(p: &egui::Painter, rect: Rect, radius: f32, color: Color32) {
    if !is_neon() {
        return;
    }
    for (grow, alpha) in [(1.5, 90u8), (4.0, 46), (7.0, 20)] {
        p.rect_stroke(
            rect.expand(grow),
            radius + grow,
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)),
            StrokeKind::Middle,
        );
    }
}

/// A rainbow strip across `rect` (moving unless calmed).
pub fn paint_rainbow(p: &egui::Painter, rect: Rect) {
    let n = 48;
    let w = rect.width() / n as f32;
    for i in 0..n {
        let r = Rect::from_min_size(pos2(rect.left() + i as f32 * w, rect.top()), vec2(w + 0.5, rect.height()));
        p.rect_filled(r, 0.0, rainbow(i as f32 / n as f32));
    }
}

/// The desktop's light/dark preference, from the freedesktop settings portal
/// (KDE, GNOME and others answer it): Some(true) = dark, Some(false) = light.
pub fn system_prefers_dark() -> Option<bool> {
    let out = crate::ctl::run_host(&[
        "busctl", "--user", "call", "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings", "ReadOne", "ss", "org.freedesktop.appearance", "color-scheme",
    ])
    .ok()?;
    // e.g. "v u 1": 1 = prefers dark, 2 = prefers light, 0 = no preference.
    match out.split_whitespace().last()? {
        "1" => Some(true),
        "2" => Some(false),
        _ => kde_scheme_is_dark(),
    }
}

/// Fallback for desktops without the portal answer: KDE's window background colour.
fn kde_scheme_is_dark() -> Option<bool> {
    let home = std::env::var_os("HOME")?;
    let text = std::fs::read_to_string(std::path::Path::new(&home).join(".config/kdeglobals")).ok()?;
    let mut in_window = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_window = l == "[Colors:Window]";
        } else if in_window && l.starts_with("BackgroundNormal=") {
            let v: Vec<f32> = l["BackgroundNormal=".len()..].split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() >= 3 {
                return Some(0.299 * v[0] + 0.587 * v[1] + 0.114 * v[2] < 128.0);
            }
        }
    }
    None
}

pub fn apply_theme(ctx: &egui::Context) {
    let mut v = if is_dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = BG();
    v.window_fill = CARD();
    v.window_stroke = Stroke::new(1.0, LINE());
    v.override_text_color = None;
    v.extreme_bg_color = pick(rgb(10, 10, 12), rgb(250, 250, 253), rgb(1, 0, 4), rgb(250, 248, 255));
    v.faint_bg_color = CARD_HI();
    v.selection.bg_fill = ACCENT_DIM();
    v.selection.stroke = Stroke::new(1.0, ACCENT());
    v.hyperlink_color = CYAN();
    v.widgets.noninteractive.bg_fill = CARD();
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT());
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE());
    v.widgets.inactive.bg_fill = CARD_HI();
    v.widgets.inactive.weak_bg_fill = CARD_HI();
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT());
    v.widgets.hovered.bg_fill = HOVER();
    v.widgets.hovered.weak_bg_fill = HOVER();
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT());
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT());
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT());
    v.widgets.active.bg_fill = ACCENT_DIM();
    v.widgets.active.weak_bg_fill = ACCENT_DIM();
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.slider_width = 260.0;
        s.text_styles.insert(egui::TextStyle::Body, FontId::proportional(14.5));
        s.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.5));
        s.text_styles.insert(egui::TextStyle::Heading, FontId::proportional(22.0));
        s.text_styles.insert(egui::TextStyle::Small, FontId::proportional(12.0));
    });
}

// ---------------------------------------------------------------------------
// Sidebar icons, drawn: a fan (all themes) and neon line icons (Neon).
// ---------------------------------------------------------------------------

/// A fan: three swept blades around a hub, inside a ring. `filled` draws solid blades
/// (Dark/Light, next to the emoji icons); otherwise outlines (Neon).
pub fn fan_icon(p: &egui::Painter, c: Pos2, r: f32, color: Color32, filled: bool, stroke: f32) {
    let at = |a: f32, d: f32| c + vec2(a.cos(), a.sin()) * d * r;
    for k in 0..3 {
        let a = k as f32 * std::f32::consts::TAU / 3.0 - 0.6;
        // A swept blade: from near the hub, out and around.
        let pts: Vec<Pos2> = [(-0.45, 0.22), (-0.25, 0.62), (0.15, 0.86), (0.45, 0.72), (0.35, 0.35), (0.15, 0.2)]
            .iter()
            .map(|(da, d)| at(a + da, *d))
            .collect();
        if filled {
            p.add(Shape::convex_polygon(pts.clone(), color, Stroke::NONE));
        } else {
            let mut closed = pts.clone();
            closed.push(pts[0]);
            p.add(Shape::line(closed, Stroke::new(stroke, color)));
        }
    }
    if filled {
        p.circle_filled(c, r * 0.2, lerp_color(color, Color32::BLACK, 0.45));
        p.circle_stroke(c, r * 0.98, Stroke::new(r * 0.12, lerp_color(color, Color32::BLACK, 0.3)));
    } else {
        p.circle_stroke(c, r * 0.18, Stroke::new(stroke, color));
        p.circle_stroke(c, r * 0.98, Stroke::new(stroke, color));
    }
}

/// The Neon colour for each page's icon.
pub fn neon_icon_color(name: &str) -> Color32 {
    match name {
        "home" | "fans" | "settings" => CYAN(),
        "performance" | "summary" => ACCENT(),
        "graphics" | "battery" => GREEN(),
        _ => AMBER(),
    }
}

/// A neon line icon for a sidebar page, with a soft glow.
pub fn neon_icon(p: &egui::Painter, name: &str, c: Pos2, r: f32, color: Color32) {
    // Each icon is drawn twice: a wide faint stroke (the glow), then the bright line.
    for (w, a) in [(5.0, 50u8), (1.8, 255)] {
        let col = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
        let s = Stroke::new(w, col);
        let pt = |x: f32, y: f32| c + vec2(x, y) * r;
        let poly = |pts: &[(f32, f32)], closed: bool| {
            let mut v: Vec<Pos2> = pts.iter().map(|(x, y)| pt(*x, *y)).collect();
            if closed {
                v.push(v[0]);
            }
            p.add(Shape::line(v, s));
        };
        match name {
            "home" => {
                poly(&[(-0.85, -0.05), (0.0, -0.85), (0.85, -0.05)], false);
                poly(&[(-0.6, -0.25), (-0.6, 0.8), (0.6, 0.8), (0.6, -0.25)], false);
                poly(&[(-0.18, 0.8), (-0.18, 0.3), (0.18, 0.3), (0.18, 0.8)], false);
            }
            "performance" => {
                // A speedometer: an arc, a needle, a hub.
                let arc: Vec<Pos2> = (0..=24)
                    .map(|i| {
                        let a = std::f32::consts::PI * (1.0 + i as f32 / 24.0);
                        pt(a.cos() * 0.85, a.sin() * 0.85 + 0.3)
                    })
                    .collect();
                p.add(Shape::line(arc, s));
                p.line_segment([pt(0.0, 0.3), pt(0.5, -0.3)], s);
                p.circle_stroke(pt(0.0, 0.3), 0.12 * r, s);
            }
            "graphics" => {
                poly(&[(-0.75, -0.5), (0.75, -0.5), (0.75, 0.5), (-0.75, 0.5)], true);
                poly(&[(-0.3, -0.2), (0.3, -0.2), (0.3, 0.2), (-0.3, 0.2)], true);
                for x in [-0.45, 0.0, 0.45] {
                    p.line_segment([pt(x, -0.5), pt(x, -0.85)], s);
                    p.line_segment([pt(x, 0.5), pt(x, 0.85)], s);
                }
            }
            "fans" => fan_icon(p, c, r, col, false, w),
            "battery" => {
                poly(&[(-0.8, -0.4), (0.6, -0.4), (0.6, 0.4), (-0.8, 0.4)], true);
                poly(&[(0.6, -0.15), (0.82, -0.15), (0.82, 0.15), (0.6, 0.15)], false);
                for x in [-0.55, -0.25, 0.05, 0.35] {
                    p.line_segment([pt(x, -0.2), pt(x, 0.2)], s);
                }
            }
            "lighting" => {
                p.circle_stroke(pt(0.0, -0.2), 0.5 * r, s);
                p.line_segment([pt(-0.22, 0.45), pt(0.22, 0.45)], s);
                p.line_segment([pt(-0.18, 0.65), pt(0.18, 0.65)], s);
                p.line_segment([pt(-0.08, 0.85), pt(0.08, 0.85)], s);
            }
            "summary" => {
                for y in [-0.55, 0.0, 0.55] {
                    p.circle_stroke(pt(-0.65, y), 0.08 * r, s);
                    p.line_segment([pt(-0.35, y), pt(0.8, y)], s);
                }
            }
            "settings" => {
                p.circle_stroke(c, 0.42 * r, s);
                p.circle_stroke(c, 0.14 * r, s);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::TAU / 8.0;
                    p.line_segment([pt(a.cos() * 0.55, a.sin() * 0.55), pt(a.cos() * 0.85, a.sin() * 0.85)], s);
                }
            }
            // The mode tiles (Performance, Fans and Graphics pages).
            "silent" => {
                // A crescent moon: the outer arc, then the inner arc back.
                let arc = |cx: f32, cy: f32, rad: f32, from: f32, to: f32| -> Vec<Pos2> {
                    (0..=20)
                        .map(|i| {
                            let a = (from + (to - from) * i as f32 / 20.0).to_radians();
                            pt(cx + a.cos() * rad, cy + a.sin() * rad)
                        })
                        .collect()
                };
                let mut v = arc(0.0, 0.0, 0.78, 14.3, 269.1);
                v.extend(arc(0.38, -0.3, 0.62, 230.8, 52.7));
                p.add(Shape::line(v, s));
            }
            "balanced" => {
                // Scales: a post on a base, a beam, and a pan hanging from each end.
                p.line_segment([pt(0.0, -0.7), pt(0.0, 0.75)], s);
                p.line_segment([pt(-0.4, 0.75), pt(0.4, 0.75)], s);
                p.line_segment([pt(-0.72, -0.5), pt(0.72, -0.5)], s);
                p.circle_stroke(pt(0.0, -0.78), 0.08 * r, s);
                for x in [-0.72, 0.72] {
                    poly(&[(x - 0.24, 0.1), (x, -0.5), (x + 0.24, 0.1)], false);
                    let bowl: Vec<Pos2> = (0..=10)
                        .map(|i| {
                            let a = std::f32::consts::PI * i as f32 / 10.0;
                            pt(x + a.cos() * 0.24, 0.1 + a.sin() * 0.16)
                        })
                        .collect();
                    p.add(Shape::line(bowl, s));
                }
            }
            "turbo" => {
                // A rocket: body with a nose, a window, fins and a flame.
                poly(&[(0.0, -0.9), (0.28, -0.45), (0.28, 0.35), (-0.28, 0.35), (-0.28, -0.45)], true);
                p.circle_stroke(pt(0.0, -0.22), 0.12 * r, s);
                poly(&[(-0.28, 0.0), (-0.55, 0.45), (-0.28, 0.35)], false);
                poly(&[(0.28, 0.0), (0.55, 0.45), (0.28, 0.35)], false);
                poly(&[(-0.14, 0.45), (0.0, 0.85), (0.14, 0.45)], false);
            }
            "eco" => {
                // A leaf: two curves from stem to tip, and the midrib.
                let curve = |c: (f32, f32)| -> Vec<Pos2> {
                    let (b, t) = ((-0.6f32, 0.6f32), (0.65f32, -0.65f32));
                    (0..=16)
                        .map(|i| {
                            let u = i as f32 / 16.0;
                            let q = |a: f32, m: f32, z: f32| (1.0 - u) * (1.0 - u) * a + 2.0 * (1.0 - u) * u * m + u * u * z;
                            pt(q(b.0, c.0, t.0), q(b.1, c.1, t.1))
                        })
                        .collect()
                };
                p.add(Shape::line(curve((-0.8, -0.7)), s));
                p.add(Shape::line(curve((0.7, 0.75)), s));
                p.line_segment([pt(-0.85, 0.85), pt(0.4, -0.4)], s);
            }
            "standard" => {
                // A lightning bolt.
                poly(&[(0.15, -0.9), (-0.45, 0.1), (-0.02, 0.1), (-0.15, 0.9), (0.45, -0.15), (0.03, -0.15)], true);
            }
            "ultimate" => {
                // A game controller: body with grips, a d-pad and two buttons.
                poly(
                    &[(-0.8, -0.35), (0.8, -0.35), (0.95, 0.45), (0.72, 0.62), (0.42, 0.3), (-0.42, 0.3), (-0.72, 0.62), (-0.95, 0.45)],
                    true,
                );
                p.line_segment([pt(-0.62, -0.02), pt(-0.28, -0.02)], s);
                p.line_segment([pt(-0.45, -0.19), pt(-0.45, 0.15)], s);
                p.circle_stroke(pt(0.38, -0.08), 0.07 * r, s);
                p.circle_stroke(pt(0.58, 0.1), 0.07 * r, s);
            }
            _ => {
                // Activity: a pulse line.
                poly(&[(-0.9, 0.1), (-0.45, 0.1), (-0.25, -0.6), (0.05, 0.7), (0.3, 0.1), (0.9, 0.1)], false);
            }
        }
    }
}

/// A card of a fixed height (the 3×3 Home grid); what doesn't fit is cut off.
pub fn card_sized<R>(ui: &mut Ui, title: &str, height: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    // Reserve exactly this much room, then draw the card inside it.
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)),
    );
    // Room for Neon's glow around the edge; anything taller than the tile is cut off.
    child.set_clip_rect(rect.expand(14.0).intersect(ui.clip_rect()));
    card(&mut child, title, |ui| {
        // Inside the reserved rect, `available_height` already ends at the tile's inner
        // bottom edge, so contents can size themselves to it.
        let bottom = rect.bottom() - 14.0;
        ui.set_clip_rect(ui.clip_rect().with_max_y(bottom + 2.0));
        let r = add(ui);
        // Fill whatever is left so every tile in a row is the same height.
        let rest = bottom - 2.0 - ui.cursor().top();
        if rest > 0.0 {
            ui.add_space(rest);
        }
        r
    })
}

/// A large value with its unit, and a few short notes beside it.
pub fn big_value(ui: &mut Ui, value: Option<String>, unit: &str, color: Color32, notes: &[String]) {
    ui.horizontal(|ui| {
        match value {
            Some(v) => {
                ui.label(RichText::new(v).size(30.0).strong().color(color));
                ui.label(RichText::new(unit).size(15.0).color(color));
            }
            None => {
                ui.label(RichText::new("–").size(30.0).color(DIM()));
            }
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for n in notes.iter().filter(|n| !n.is_empty()) {
                ui.label(RichText::new(n).size(12.5).color(DIM()));
            }
        });
    });
}

pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut frame = egui::Frame::new().fill(CARD()).stroke(Stroke::new(1.0, LINE()));
    if is_neon() {
        // Neon tube: a bright edge with a soft cyan glow around it.
        let c = CYAN();
        frame = frame
            .stroke(Stroke::new(1.2, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 120)))
            .shadow(egui::Shadow {
                offset: [0, 0],
                blur: 18,
                spread: 0,
                color: Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 34),
            });
    }
    frame
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title.to_uppercase()).size(12.0).color(DIM()).strong());
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}

pub fn page_title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(26.0).strong().color(TEXT()));
    ui.label(RichText::new(subtitle).color(DIM()));
    if is_neon() {
        // A moving rainbow underline in Neon.
        ui.add_space(2.0);
        // Full width of the page, edge to edge.
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 3.0), Sense::hover());
        paint_rainbow(ui.painter(), rect);
    }
    ui.add_space(10.0);
}

/// A ring gauge with a big value in the middle.
pub fn gauge(ui: &mut Ui, size: f32, frac: f32, value: &str, unit: &str, label: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(size, size + 18.0), Sense::hover());
    let p = ui.painter();
    let c = pos2(rect.center().x, rect.top() + size / 2.0);
    let r = size / 2.0 - 8.0;
    let start = std::f32::consts::PI * 0.75;
    let sweep = std::f32::consts::PI * 1.5;
    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 48;
        (0..=n)
            .map(|i| {
                let a = from + (to - from) * i as f32 / n as f32;
                c + vec2(a.cos(), a.sin()) * r
            })
            .collect()
    };
    p.add(Shape::line(arc(start, start + sweep), Stroke::new(8.0, CARD_HI())));
    let f = frac.clamp(0.0, 1.0);
    if f > 0.001 && is_neon() {
        // Neon: a glow under the arc and a cyan → magenta sweep along it.
        let pts = arc(start, start + sweep * f);
        let (a, b) = (CYAN(), ACCENT());
        for w in [20.0, 14.0] {
            p.add(Shape::line(pts.clone(), Stroke::new(w, Color32::from_rgba_unmultiplied(b.r(), b.g(), b.b(), 22))));
        }
        let n = pts.len() - 1;
        for (i, seg) in pts.windows(2).enumerate() {
            let t = (i as f32 / n as f32) * f;
            p.line_segment([seg[0], seg[1]], Stroke::new(8.0, lerp_color(a, b, t)));
        }
    } else if f > 0.001 {
        p.add(Shape::line(arc(start, start + sweep * f), Stroke::new(8.0, color)));
    }
    p.text(c + vec2(0.0, -4.0), Align2::CENTER_CENTER, value, FontId::proportional(size * 0.22), TEXT());
    p.text(c + vec2(0.0, size * 0.16), Align2::CENTER_CENTER, unit, FontId::proportional(12.0), DIM());
    p.text(
        pos2(c.x, rect.bottom() - 6.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        DIM(),
    );
}

/// A small history line.
pub fn sparkline(ui: &mut Ui, values: &[f32], lo: f32, hi: f32, color: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, GRAPH_BG());
    if values.len() < 2 {
        return;
    }
    let n = values.len();
    let pts: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = rect.left() + rect.width() * i as f32 / (n - 1) as f32;
            let t = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
            pos2(x, rect.bottom() - 4.0 - t * (rect.height() - 8.0))
        })
        .collect();
    let mut fill = pts.clone();
    fill.push(pos2(rect.right(), rect.bottom()));
    fill.push(pos2(rect.left(), rect.bottom()));
    let faded = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
    for w in pts.windows(2) {
        p.add(Shape::convex_polygon(
            vec![w[0], w[1], pos2(w[1].x, rect.bottom()), pos2(w[0].x, rect.bottom())],
            faded,
            Stroke::NONE,
        ));
    }
    if is_neon() {
        let glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 40);
        p.add(Shape::line(pts.clone(), Stroke::new(7.0, glow)));
    }
    p.add(Shape::line(pts, Stroke::new(2.0, color)));
}

/// A temperature history with a degree scale: gridlines labelled 40°…100°, the line in
/// `color`, and a dot with the current value at its right end.
/// Returns the plotting area, for `overlay_line`.
pub fn temp_graph(ui: &mut Ui, values: &[f32], color: Color32, height: f32) -> Rect {
    history_graph(ui, values, (30.0, 100.0), &[40.0, 60.0, 80.0, 100.0], "°", "°C", color, height, Line::Heat)
}

/// How a history line is coloured.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Line {
    /// In Neon, a moving rainbow (otherwise the given colour).
    Rainbow,
    /// Each point by its temperature: cool cyan, green, yellow, orange, hot red.
    Heat,
}

/// A history line with a labelled scale: `range` is the bottom and top, `ticks` get gridlines
/// labelled with `tick_unit`, and the current value is marked at the right end with `unit`.
/// With `rgb` (and the Neon theme), the line is a moving rainbow instead of `color`.
#[allow(clippy::too_many_arguments)]
pub fn history_graph(
    ui: &mut Ui,
    values: &[f32],
    range: (f32, f32),
    ticks: &[f32],
    tick_unit: &str,
    unit: &str,
    color: Color32,
    height: f32,
    line: Line,
) -> Rect {
    let rgb = line == Line::Rainbow && is_neon();
    let heat = line == Line::Heat;
    let (lo, hi) = range;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, GRAPH_BG());
    // Room on the left for the scale and on the right for the current-value label.
    let plot = Rect::from_min_max(rect.min + vec2(40.0, 8.0), rect.max - vec2(54.0, 8.0));
    let y_of = |t: f32| plot.bottom() - ((t - lo) / (hi - lo)).clamp(0.0, 1.0) * plot.height();
    for t in ticks {
        let y = y_of(*t);
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, GRID()));
        p.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{t:.0}{tick_unit}"), FontId::proportional(11.0), DIM());
    }
    if values.len() < 2 {
        if let Some(v) = values.last() {
            let at = pos2(plot.right(), y_of(*v));
            let c = if heat { heat_color(*v) } else { color };
            p.circle_filled(at, 4.5, c);
            p.text(at + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{v:.0}{unit}"), FontId::proportional(13.0), c);
        }
        return plot;
    }
    let n = values.len();
    let pts: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(plot.left() + plot.width() * i as f32 / (n - 1) as f32, y_of(*v)))
        .collect();
    // The colour along the line: fixed, or (rgb) a rainbow across it that keeps moving.
    let at = |i: usize| -> Color32 {
        if rgb {
            rainbow(i as f32 / n as f32 * 0.8)
        } else if heat {
            heat_color(values[i])
        } else {
            color
        }
    };
    let alpha = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    for (i, w) in pts.windows(2).enumerate() {
        p.add(Shape::convex_polygon(
            vec![w[0], w[1], pos2(w[1].x, plot.bottom()), pos2(w[0].x, plot.bottom())],
            alpha(at(i), 26),
            Stroke::NONE,
        ));
    }
    if rgb || heat {
        for (i, w) in pts.windows(2).enumerate() {
            p.line_segment([w[0], w[1]], Stroke::new(7.0, alpha(at(i), 44)));
        }
        for (i, w) in pts.windows(2).enumerate() {
            p.line_segment([w[0], w[1]], Stroke::new(2.4, at(i)));
        }
    } else {
        if is_neon() {
            p.add(Shape::line(pts.clone(), Stroke::new(7.0, alpha(color, 40))));
        }
        p.add(Shape::line(pts.clone(), Stroke::new(2.0, color)));
    }
    // Current value: a dot at the end of the line with its number beside it.
    let end = at(n - 1);
    let last = *pts.last().unwrap_or(&plot.right_center());
    let now = values[n - 1];
    p.circle_filled(last, 4.5, end);
    p.circle_stroke(last, 4.5, Stroke::new(1.5, GRAPH_BG()));
    p.text(last + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{now:.0}{unit}"), FontId::proportional(13.0), end);
    plot
}

/// One history in a `multi_graph`.
pub struct Series<'a> {
    pub values: &'a [f32],
    pub color: Color32,
    /// Drawn faintly (a device that's asleep or off).
    pub dim: bool,
}

/// Several histories on one scale (`range`, gridlines at `ticks` labelled with `tick_unit`),
/// each in its own colour, newest at the right; each line's current value is marked with a dot.
pub fn multi_graph(ui: &mut Ui, series: &[Series], range: (f32, f32), ticks: &[f32], tick_unit: &str, height: f32) {
    let (lo, hi) = range;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, GRAPH_BG());
    let plot = Rect::from_min_max(rect.min + vec2(48.0, 8.0), rect.max - vec2(14.0, 8.0));
    let y_of = |t: f32| plot.bottom() - ((t - lo) / (hi - lo)).clamp(0.0, 1.0) * plot.height();
    for t in ticks {
        let y = y_of(*t);
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, GRID()));
        p.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{t:.0}{tick_unit}"), FontId::proportional(11.0), DIM());
    }
    let n = series.iter().map(|s| s.values.len()).max().unwrap_or(0);
    if n < 2 {
        return;
    }
    let alpha = |c: Color32, a: u8| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    // Faint ones first, so an active line is drawn on top.
    let mut order: Vec<&Series> = series.iter().collect();
    order.sort_by_key(|s| !s.dim);
    for s in order {
        let m = s.values.len();
        if m == 0 {
            continue;
        }
        // Newest at the right edge; a shorter history starts later.
        let pts: Vec<Pos2> = s
            .values
            .iter()
            .enumerate()
            .map(|(i, v)| pos2(plot.right() - plot.width() * (m - 1 - i) as f32 / (n - 1) as f32, y_of(*v)))
            .collect();
        let c = if s.dim { alpha(s.color, 70) } else { s.color };
        if !s.dim {
            for w in pts.windows(2) {
                p.add(Shape::convex_polygon(
                    vec![w[0], w[1], pos2(w[1].x, plot.bottom()), pos2(w[0].x, plot.bottom())],
                    alpha(s.color, 16),
                    Stroke::NONE,
                ));
            }
            if is_neon() {
                p.add(Shape::line(pts.clone(), Stroke::new(7.0, alpha(s.color, 40))));
            }
        }
        p.add(Shape::line(pts.clone(), Stroke::new(if s.dim { 1.5 } else { 2.2 }, c)));
        if let Some(last) = pts.last() {
            p.circle_filled(*last, 4.0, c);
            p.circle_stroke(*last, 4.0, Stroke::new(1.5, GRAPH_BG()));
        }
    }
}

/// iOS-style switch.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let size = vec2(44.0, 24.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let bg = lerp_color(CARD_HI(), ACCENT(), t);
    let p = ui.painter();
    p.rect_filled(rect, 12.0, bg);
    p.rect_stroke(rect, 12.0, Stroke::new(1.0, LINE()), StrokeKind::Inside);
    let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
    p.circle_filled(pos2(x, rect.center().y), 9.0, Color32::WHITE);
    resp
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A row of equal buttons where one is chosen.
pub fn segmented(ui: &mut Ui, options: &[&str], selected: &mut usize) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, o) in options.iter().enumerate() {
            let sel = *selected == i;
            let b = egui::Button::new(RichText::new(*o).color(if sel { Color32::WHITE } else { TEXT() }))
                .fill(if sel { ACCENT() } else { CARD_HI() })
                .min_size(vec2(72.0, 30.0));
            if ui.add(b).clicked() && !sel {
                *selected = i;
                changed = true;
            }
        }
    });
    changed
}

/// Large selectable tile (performance and graphics modes).
pub fn mode_tile(ui: &mut Ui, width: f32, icon: &str, title: &str, desc: &str, selected: bool, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 132.0), Sense::click());
    let p = ui.painter();
    let hovered = resp.hovered();
    let fill = if selected {
        lerp_color(CARD(), color, 0.18)
    } else if hovered {
        CARD_HI()
    } else {
        CARD()
    };
    // Neon: the chosen tile glows magenta, a hovered one cyan.
    if selected {
        glow_rect(p, rect, 10.0, ACCENT());
    } else if hovered {
        glow_rect(p, rect, 10.0, CYAN());
    }
    p.rect_filled(rect, 10.0, fill);
    let edge = if is_neon() && selected { ACCENT() } else if selected { color } else { LINE() };
    p.rect_stroke(rect, 10.0, Stroke::new(if selected { 2.0 } else { 1.0 }, edge), StrokeKind::Inside);
    // Neon draws its own glowing line icons; the other themes use the emoji.
    let neon_name = match icon {
        "🌙" => Some("silent"),
        "⚖" => Some("balanced"),
        "🚀" => Some("turbo"),
        "🍃" => Some("eco"),
        "⚡" => Some("standard"),
        "🎮" => Some("ultimate"),
        _ => None,
    };
    match neon_name.filter(|_| is_neon()) {
        Some(n) => neon_icon(p, n, rect.left_top() + vec2(34.0, 34.0), 15.0, color),
        None => {
            p.text(rect.left_top() + vec2(18.0, 18.0), Align2::LEFT_TOP, icon, FontId::proportional(28.0), color);
        }
    }
    p.text(rect.left_top() + vec2(18.0, 62.0), Align2::LEFT_TOP, title, FontId::proportional(19.0), TEXT());
    let galley = p.layout(desc.to_string(), FontId::proportional(12.5), DIM(), width - 36.0);
    p.galley(rect.left_top() + vec2(18.0, 90.0), galley, DIM());
    if selected {
        p.circle_filled(rect.right_top() + vec2(-20.0, 20.0), 6.0, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Editable 8-point fan curve. Returns true when a point was moved.
pub fn curve_editor(
    ui: &mut Ui,
    id: &str,
    curve: &mut Curve,
    live_temp: Option<f32>,
    color: Color32,
    enabled: bool,
) -> bool {
    let w = ui.available_width();
    let h = 230.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let plot = Rect::from_min_max(rect.min + vec2(40.0, 10.0), rect.max - vec2(12.0, 26.0));
    let to_screen = |t: f32, d: f32| {
        pos2(
            plot.left() + (t - 20.0) / 80.0 * plot.width(),
            plot.bottom() - d / 100.0 * plot.height(),
        )
    };
    let p = ui.painter().clone();
    p.rect_filled(rect, 8.0, GRAPH_BG());
    for i in 0..=4 {
        let d = i as f32 * 25.0;
        let y = to_screen(20.0, d).y;
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0, LINE()));
        p.text(pos2(plot.left() - 6.0, y), Align2::RIGHT_CENTER, format!("{d:.0}%"), FontId::proportional(11.0), DIM());
    }
    for t in (20..=100).step_by(10) {
        let x = to_screen(t as f32, 0.0).x;
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0, GRID()));
        p.text(pos2(x, plot.bottom() + 6.0), Align2::CENTER_TOP, format!("{t}°"), FontId::proportional(11.0), DIM());
    }
    if let Some(t) = live_temp {
        let x = to_screen(t.clamp(20.0, 100.0), 0.0).x;
        p.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.5, AMBER()));
        p.text(pos2(x + 4.0, plot.top() + 2.0), Align2::LEFT_TOP, format!("now {t:.0}°C"), FontId::proportional(11.0), AMBER());
    }

    let line_color = if enabled { color } else { DIM() };
    let pts: Vec<Pos2> = curve.iter().map(|(t, d)| to_screen(*t as f32, *d as f32)).collect();
    let mut fill = vec![pos2(pts[0].x, plot.bottom())];
    fill.extend(pts.iter().copied());
    fill.push(pos2(pts[7].x, plot.bottom()));
    for w2 in pts.windows(2) {
        p.add(Shape::convex_polygon(
            vec![w2[0], w2[1], pos2(w2[1].x, plot.bottom()), pos2(w2[0].x, plot.bottom())],
            Color32::from_rgba_unmultiplied(line_color.r(), line_color.g(), line_color.b(), 22),
            Stroke::NONE,
        ));
    }
    if is_neon() {
        let g = Color32::from_rgba_unmultiplied(line_color.r(), line_color.g(), line_color.b(), 44);
        p.add(Shape::line(pts.clone(), Stroke::new(9.0, g)));
    }
    p.add(Shape::line(pts.clone(), Stroke::new(2.5, line_color)));

    // Each point sits on a fixed temperature column and only moves up or down (fan speed).
    // The whole column is the handle, so a point is easy to grab even near another one.
    let mut changed = false;
    let col_w = (plot.width() / 8.0 * 0.8).clamp(14.0, 48.0);
    for i in 0..8 {
        let pid = ui.make_persistent_id((id, i));
        let hit = Rect::from_center_size(
            pos2(pts[i].x, plot.center().y),
            vec2(col_w, plot.height() + 16.0),
        );
        let r = ui.interact(hit, pid, Sense::click_and_drag());
        if r.dragged() || r.clicked() {
            if let Some(pos) = r.interact_pointer_pos() {
                let d = ((plot.bottom() - pos.y) / plot.height() * 100.0).round().clamp(0.0, 100.0) as u8;
                if d != curve[i].1 {
                    curve[i].1 = d;
                    // Keep the curve rising: points after this one are at least as fast,
                    // points before it at most as fast.
                    for j in i + 1..8 {
                        curve[j].1 = curve[j].1.max(d);
                    }
                    for j in 0..i {
                        curve[j].1 = curve[j].1.min(d);
                    }
                    changed = true;
                }
            }
        }
        let active = r.hovered() || r.dragged();
        if active {
            p.rect_filled(
                Rect::from_center_size(pos2(pts[i].x, plot.center().y), vec2(col_w, plot.height())),
                4.0,
                Color32::from_rgba_unmultiplied(TEXT().r(), TEXT().g(), TEXT().b(), 10),
            );
        }
        p.circle_filled(pts[i], if active { 7.5 } else { 5.5 }, if active { TEXT() } else { line_color });
        p.circle_stroke(pts[i], if active { 7.5 } else { 5.5 }, Stroke::new(2.0, BG()));
        if active {
            p.text(
                pts[i] + vec2(0.0, -14.0),
                Align2::CENTER_BOTTOM,
                format!("{}°C · {}%", curve[i].0, curve[i].1),
                FontId::proportional(12.0),
                TEXT(),
            );
            r.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        }
    }
    changed
}

pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let c = egui::ecolor::Hsva::new(h.rem_euclid(1.0), s, v, 1.0);
    Color32::from(c)
}

/// A stylised keyboard that previews the lighting effect.
pub fn keyboard_preview(ui: &mut Ui, color_at: impl Fn(f32) -> Color32, brightness: f32) {
    let w = ui.available_width().min(620.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 200.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 12.0, Color32::from_rgb(16, 16, 20));
    p.rect_stroke(rect, 12.0, Stroke::new(1.0, LINE()), StrokeKind::Inside);
    let inner = rect.shrink(16.0);
    let rows: [&[f32]; 5] = [
        &[1.0; 14],
        &[1.5, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.5],
        &[1.8, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.2],
        &[2.3, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.7],
        &[1.2, 1.2, 1.2, 6.0, 1.2, 1.2, 1.0, 1.0, 1.0],
    ];
    let gap = 4.0;
    let rh = (inner.height() - gap * 4.0) / 5.0;
    for (ri, row) in rows.iter().enumerate() {
        let total: f32 = row.iter().sum();
        let unit = (inner.width() - gap * (row.len() as f32 - 1.0)) / total;
        let mut x = inner.left();
        let y = inner.top() + ri as f32 * (rh + gap);
        for k in row.iter() {
            let kw = k * unit;
            let r = Rect::from_min_size(pos2(x, y), vec2(kw, rh));
            let frac = (r.center().x - inner.left()) / inner.width();
            let lit = color_at(frac);
            let glow = lerp_color(Color32::from_rgb(30, 30, 36), lit, brightness);
            p.rect_filled(r, 4.0, Color32::from_rgb(28, 28, 34));
            p.rect_stroke(r, 4.0, Stroke::new(1.5, glow), StrokeKind::Inside);
            p.rect_filled(r.shrink(3.0), 3.0, lerp_color(Color32::from_rgb(28, 28, 34), lit, brightness * 0.35));
            x += kw + gap;
        }
    }
}

pub fn stat_row(ui: &mut Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(DIM()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(value).color(TEXT()));
        });
    });
}

pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(lerp_color(CARD(), color, 0.16))
        .stroke(Stroke::new(1.0, lerp_color(LINE(), color, 0.5)))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(13.0).color(TEXT()));
        });
}

pub fn accent_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
            .fill(ACCENT())
            .min_size(vec2(120.0, 34.0)),
    )
}
