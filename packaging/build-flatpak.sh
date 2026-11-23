#!/usr/bin/env bash
# Build ArcheryCrate as a Flatpak and write a single-file bundle: packaging/out/ArcheryCrate.flatpak
#
# Needs: flatpak-builder (or `flatpak install flathub org.flatpak.Builder`),
#        python3 with the aiohttp and tomlkit modules (for the crate list),
#        flathub: org.freedesktop.Sdk//26.08 and org.freedesktop.Sdk.Extension.rust-stable//26.08
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
FP="$HERE/flatpak"
ID=io.github.archerycrate.ArcheryCrate
TOOLS="$HERE/tools"
OUT="$HERE/out"

if command -v flatpak-builder >/dev/null; then
    BUILDER=(flatpak-builder)
elif flatpak info org.flatpak.Builder >/dev/null 2>&1; then
    BUILDER=(flatpak run org.flatpak.Builder)
else
    echo "flatpak-builder is missing. Install it with:  flatpak install flathub org.flatpak.Builder" >&2
    exit 1
fi

echo "==> Making sure the SDK is installed"
flatpak install --user -y --noninteractive flathub \
    org.freedesktop.Sdk//26.08 org.freedesktop.Platform//26.08 \
    org.freedesktop.Sdk.Extension.rust-stable//26.08 || true

GEN="$TOOLS/flatpak-cargo-generator.py"
if [ ! -f "$GEN" ]; then
    echo "==> Downloading flatpak-cargo-generator"
    mkdir -p "$TOOLS"
    curl -fL --proto '=https' -o "$GEN" \
        "https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py"
fi

echo "==> Listing crates for the offline build"
python3 "$GEN" "$ROOT/Cargo.lock" -o "$FP/cargo-sources.json"

echo "==> Building"
cd "$FP"
"${BUILDER[@]}" --user --force-clean --install-deps-from=flathub --repo=repo build-dir "$ID.yml"

echo "==> Writing bundle"
mkdir -p "$OUT"
flatpak build-bundle repo "$OUT/ArcheryCrate.flatpak" "$ID" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
echo "Done: $OUT/ArcheryCrate.flatpak"
echo "Install it with:  flatpak install --user $OUT/ArcheryCrate.flatpak"
