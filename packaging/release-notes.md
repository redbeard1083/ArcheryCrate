ArcheryCrate is a control center for ASUS laptops on Linux: performance modes and power limits,
graphics modes, fan curves, battery charge limit and on-battery automation, keyboard lighting,
and a live dashboard. Developed on a 2022 ROG Zephyrus G14 (GA402RJ) with KDE Plasma on Wayland.

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
