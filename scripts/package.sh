#!/usr/bin/env bash
# 把 release 二进制打成分发包（macOS / Linux），产物放在 dist/。Windows 的 zip 在 CI 里直接用 PowerShell 打。
#
# 用法：
#   cargo build --release --locked -p pc-specs
#   scripts/package.sh <label>
#
# label 只用来命名产物，也决定打哪种包：macos-arm64 → .dmg；linux-amd64 / linux-arm64 → .tar.gz + .deb。
set -euo pipefail

label="${1:?usage: scripts/package.sh <macos-arm64|linux-amd64|linux-arm64>}"
cd "$(dirname "$0")/.."

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
bin="target/release/pc-specs"
[ -x "$bin" ] || { echo "missing $bin — run 'cargo build --release --locked -p pc-specs' first" >&2; exit 1; }

mkdir -p dist

case "$label" in
  macos-*)
    app="dist/PC Specs.app"
    rm -rf "$app" dist/dmg-root
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$bin" "$app/Contents/MacOS/pc-specs"
    cp assets/icons/icon.icns "$app/Contents/Resources/icon.icns"
    sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"
    plutil -lint "$app/Contents/Info.plist"
    # 没有 Developer ID 时做 ad-hoc 签名，保证 bundle 完整性校验通过。
    # 公开分发还需要 Developer ID 签名 + 公证，见 README。
    codesign --force --sign - "$app"
    mkdir -p dist/dmg-root
    cp -R "$app" dist/dmg-root/
    ln -s /Applications dist/dmg-root/Applications
    hdiutil create -volname "PC Specs" -srcfolder dist/dmg-root -ov -format UDZO \
      "dist/PC-Specs-$version-$label.dmg"
    rm -rf dist/dmg-root "$app"
    ;;

  linux-*)
    deb_arch=amd64
    [ "$label" = "linux-arm64" ] && deb_arch=arm64
    id="com.xwzy.pc-specs"

    # 绿色版：解压后直接运行 ./pc-specs。
    stage="dist/pc-specs-$version-$label"
    rm -rf "$stage"
    mkdir -p "$stage"
    install -m 0755 "$bin" "$stage/pc-specs"
    install -m 0644 "packaging/linux/$id.desktop" "$stage/$id.desktop"
    install -m 0644 "assets/icons/128x128@2x.png" "$stage/$id.png"
    tar -czf "dist/pc-specs-$version-$label.tar.gz" -C dist "pc-specs-$version-$label"
    rm -rf "$stage"

    # .deb：装到 /usr/bin，并带桌面入口与图标。
    root="dist/deb-root"
    rm -rf "$root"
    install -D -m 0755 "$bin" "$root/usr/bin/pc-specs"
    install -D -m 0644 "packaging/linux/$id.desktop" "$root/usr/share/applications/$id.desktop"
    install -D -m 0644 assets/icons/32x32.png "$root/usr/share/icons/hicolor/32x32/apps/$id.png"
    install -D -m 0644 assets/icons/128x128.png "$root/usr/share/icons/hicolor/128x128/apps/$id.png"
    install -D -m 0644 "assets/icons/128x128@2x.png" "$root/usr/share/icons/hicolor/256x256/apps/$id.png"
    mkdir -p "$root/DEBIAN"
    # GPUI 在运行时 dlopen Vulkan / Wayland / xkbcommon，dpkg-shlibdeps 看不到，所以手写依赖。
    cat > "$root/DEBIAN/control" <<EOF
Package: pc-specs
Version: $version
Section: utils
Priority: optional
Architecture: $deb_arch
Maintainer: pc-specs contributors <noreply@users.noreply.github.com>
Depends: libvulkan1, libxkbcommon-x11-0, libwayland-client0, libfontconfig1, libx11-xcb1
Recommends: mesa-vulkan-drivers
Description: PC configuration viewer and live monitor
 Shows CPU, GPU, memory, storage, network and sensor details of this
 computer, with live charts, a tray icon and a local JSON API.
EOF
    dpkg-deb --root-owner-group --build "$root" "dist/pc-specs_${version}_${deb_arch}.deb"
    rm -rf "$root"
    ;;

  *)
    echo "unsupported label: $label" >&2
    exit 2
    ;;
esac

ls -la dist
