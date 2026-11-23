# ArcheryCrate

A control center for ASUS laptops on Linux, written in Rust (eframe/egui), built on top of
**asusctl** and **Cardwire**. Developed and tested on a 2022 ROG Zephyrus G14 (GA402RJ,
Ryzen 9 6900HS + Radeon RX 6700S) running CachyOS with KDE Plasma (Wayland).

## What it does

| Page        | What's there                                                                                       |
|-------------|----------------------------------------------------------------------------------------------------|
| Home        | CPU, discrete-GPU and integrated-GPU load (with clock, power and history), temperatures, memory, fans, free space on each drive, battery. A detailed scrolling layout or a 3×3 grid that fits the window. |
| Performance | Silent / Balanced / Turbo modes; APU and platform power limits (asus-armoury)                     |
| Graphics    | Eco / Standard (through Cardwire, instant both ways) and Ultimate (MUX, needs a restart)          |
| Fans        | Pick a mode; drag the 8 points (fixed 30–100 °C columns) to make its fan curve yours; reset to stock |
| Battery     | Charge limit, one-time full charge, health; "on battery" automation: Silent mode, silent fans, Eco, 60 Hz, screen brightness, keyboard light off or dimmed |
| Lighting    | Keyboard effect, colours, speed, brightness, when it is lit                                         |
| Summary     | Everything the app manages, as it stands now, in one list                                          |
| Settings    | asusctl / Cardwire setup ("Install for me"), start at login, tray, theme (Follow system / Dark / Light / Neon), Home layout, panel overdrive, boot sound |
| Activity    | Every change made and what the system answered, including changes made by the tray icon          |

The tray icon (its own small process, so closing the window keeps it) offers Open, performance
modes, graphics modes and Quit. `--hidden` starts only the tray; "start at login" uses this.

## How changes are made

1. **asusctl** (and **Cardwire** for Eco/Standard) when installed. asusctl's printed errors count
   as failures even when it exits with 0.
2. The keyboard light goes through **logind** (no password).
3. Otherwise a **direct write to the kernel's settings files**, which asks for your password (pkexec).

Readings come straight from `/sys`, so the dashboard works without asusctl.

## Installing what it needs

Settings → *asusctl setup* detects your Linux and offers **Install for me**. On CachyOS/Arch:

    sudo pacman -S --needed asusctl cardwire && sudo systemctl enable --now asusd cardwired

## Building

    cargo build --release                 # the program: target/release/archerycrate
    packaging/build-appimage.sh           # packaging/out/ArcheryCrate-x86_64.AppImage
    packaging/build-flatpak.sh            # packaging/out/ArcheryCrate.flatpak (needs flatpak-builder)

The Flatpak reaches asusctl on the host through `flatpak-spawn --host`
(permission `--talk-name=org.freedesktop.Flatpak`); it hasn't been built and tested yet.

## Options

    archerycrate [--hidden] [--page <name>] [--theme system|dark|light|neon] [--screenshot <file.png>]
