#!/usr/bin/env bash
# Build ArcheryCrate as a single-file AppImage: packaging/out/ArcheryCrate-x86_64.AppImage
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
ID=io.github.archerycrate.ArcheryCrate
ARCH="${ARCH:-x86_64}"
TOOLS="$HERE/tools"
OUT="$HERE/out"
APPDIR="$OUT/ArcheryCrate.AppDir"

export PATH="$HOME/.cargo/bin:$PATH"

echo "==> Building release binary"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

echo "==> Assembling AppDir"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/scalable/apps" "$APPDIR/usr/share/metainfo"
install -m755 "$ROOT/target/release/archerycrate" "$APPDIR/usr/bin/archerycrate"
install -m644 "$HERE/$ID.desktop" "$APPDIR/usr/share/applications/$ID.desktop"
install -m644 "$HERE/$ID.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/$ID.svg"
install -m644 "$HERE/$ID.metainfo.xml" "$APPDIR/usr/share/metainfo/$ID.appdata.xml"
cp "$HERE/$ID.desktop" "$APPDIR/$ID.desktop"
cp "$HERE/$ID.svg" "$APPDIR/$ID.svg"
# appimagetool wants a PNG folder icon (.DirIcon); make one from the SVG when possible.
if command -v rsvg-convert >/dev/null 2>&1; then
    mkdir -p "$APPDIR/usr/share/icons/hicolor/256x256/apps"
    rsvg-convert -w 256 -h 256 "$HERE/$ID.svg" -o "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png"
    cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/$ID.png" "$APPDIR/$ID.png"
    ln -sf "$ID.png" "$APPDIR/.DirIcon"
else
    ln -sf "$ID.svg" "$APPDIR/.DirIcon"
fi
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/archerycrate" "$@"
EOF
chmod +x "$APPDIR/AppRun"

# Graphics, Wayland/X11 and D-Bus libraries are loaded from the host on purpose
# (bundling Mesa/Vulkan drivers breaks on other machines).

TOOL="$TOOLS/appimagetool-$ARCH.AppImage"
if [ ! -x "$TOOL" ]; then
    echo "==> Downloading appimagetool"
    mkdir -p "$TOOLS"
    curl -fL --proto '=https' -o "$TOOL" \
        "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
    chmod +x "$TOOL"
fi

echo "==> Packing AppImage"
mkdir -p "$OUT"
FINAL="$OUT/ArcheryCrate-$ARCH.AppImage"
# Pack under a temporary name, then move it into place: a copy that's running right now
# can't be overwritten ("Text file busy"), but it can be replaced — it keeps running from
# the old file and the next start uses the new one.
TMP="$OUT/.ArcheryCrate-$ARCH.AppImage.new"
rm -f "$TMP"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$TOOL" --no-appstream "$APPDIR" "$TMP"
mv -f "$TMP" "$FINAL"
echo "Done: $FINAL"
