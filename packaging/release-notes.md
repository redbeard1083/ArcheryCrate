ArcheryCrate is a control center for ASUS laptops on Linux: performance modes and power limits,
graphics modes, fan curves, battery charge limit and on-battery automation, keyboard lighting,
and a live dashboard. Developed on a 2022 ROG Zephyrus G14 (GA402RJ) with KDE Plasma on Wayland.

## What's new in 0.4

- **A new Home page.** The laptop's name is the heading; one combined graph shows CPU,
  discrete-GPU and integrated-GPU load (an idle graphics card reads "Asleep"); temperatures
  show their fan speeds; fans and memory each get a combined graph; the battery shows the time
  left. Graphs cover the last 1, 5 or 15 minutes (Settings).
- **A richer status bar:** CPU and GPU temperature, fan speeds, battery, RAM and graphics-memory
  use, and a short "Settings applied" note.
- **Themes:** a new **Light Neon** theme; line icons in every theme (glowing in Neon, flat in
  Dark and Light); a **Bold rainbow graphs** switch for any theme.
- **Small screens:** Settings › Interface size (Auto fits 720p screens and small windows).
- **On battery:** turn panel overdrive off; "Keyboard light off" and "Dim the keyboard light"
  now switch each other off.
- **Graphics:** while a switch waits for the restart, only the chosen mode is ticked, and
  picking the running mode again takes the switch back. The restart question offers Restart
  Now, Restart Later and Cancel.
- **Fixes:** the Settings cards no longer overlap in narrow windows; the battery tile on the
  3×3 Home fits.

## What's new in 0.3

- **Fix: Silent could be raised to full power.** In 0.2, leaving a mode that had its own power
  limits could write the maximum limits into the next mode. That step is gone, and
  "Back to stock" now hands the mode back to the laptop's own limits.
- **Version in Settings**, with a link to new releases.
- **Darker Neon theme** so the bright parts stand out, plus one colour scale everywhere:
  temperatures go cyan → green → yellow → orange → red, and memory, storage and battery turn
  amber, then red, only when they need attention.
- **Rainbow speed** slider in Settings (Neon).
- **Restart question as a notification.** Switching to or from Ultimate in the tray menu asks
  with a desktop notification: restart now, restart later, or cancel.
- **Eco on battery is skipped while a monitor is on the HDMI port**, because that monitor
  keeps the graphics card on anyway.

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
