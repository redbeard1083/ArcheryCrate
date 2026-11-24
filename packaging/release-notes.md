ArcheryCrate is a control center for ASUS laptops on Linux: performance modes and power limits,
graphics modes, fan curves, battery charge limit and on-battery automation, keyboard lighting,
and a live dashboard. Developed on a 2022 ROG Zephyrus G14 (GA402RJ) with KDE Plasma on Wayland.

## What's new in 0.2

- **Power limits per mode.** Silent, Balanced and Turbo each keep their own limits (plugged in
  and on battery separately). The sliders follow the active mode, apply when you let go, show
  a mark for the stock setting, and have a "Back to stock" button.
- **Plain names.** "APU sustained power" is now *Processor power limit* and "Platform power"
  is *Combined power limit*, each with a short explanation.
- **Ultimate mode** draws the window on the discrete GPU, so the integrated GPU stays idle.
- **Charger and external monitor fixes.** No more freeze when the charger and an HDMI
  monitor are plugged in together, and the monitor comes back on its own after leaving Eco.

## Download

- **ArcheryCrate-x86_64.AppImage**: download, make it executable
  (`chmod +x ArcheryCrate-x86_64.AppImage`, or Properties → Permissions in your file manager)
  and run it. On first start it adds itself to your app launcher.
- **ArcheryCrate.flatpak** (when attached): `flatpak install --user ArcheryCrate.flatpak`.
  The Flatpak is new and less tested than the AppImage.

## Needs

- **asusctl** with its service (asusd) running.
- **Cardwire** for switching between Eco and Standard graphics instantly (optional).

Settings → *asusctl setup* can install both for you on Arch-based systems such as CachyOS.
On CachyOS/Arch:

    sudo pacman -S --needed asusctl cardwire && sudo systemctl enable --now asusd cardwired
