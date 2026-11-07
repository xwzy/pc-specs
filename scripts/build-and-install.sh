#!/usr/bin/env bash
# 一键编译并安装到本机。
#
#   scripts/build-and-install.sh                      编译 release，装好并启动
#   scripts/build-and-install.sh --no-launch          只安装，不启动
#   scripts/build-and-install.sh --dir ~/Applications 装到别的目录（macOS 是 .app 所在目录，Linux 是前缀）
#
# macOS：装成 PC Specs.app，默认放 /Applications，没有写权限就退到 ~/Applications。
# Linux：装到 ~/.local（二进制、桌面入口、图标），不需要 root。
# 正在运行的实例会先退出，装完再启动新的。
set -euo pipefail

cd "$(dirname "$0")/.."

launch=1
dest=""
while [ $# -gt 0 ]; do
  case "$1" in
    --no-launch) launch=0 ;;
    --dir)
      dest="${2:?--dir needs a path}"
      shift
      ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown option: $1 (try --help)" >&2
      exit 2
      ;;
  esac
  shift
done

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
bin="target/release/pc-specs"

# 先让正在跑的实例退出：单实例锁还在的话，新启动的会去叫醒旧的然后自己退出。
stop_running() {
  pgrep -x pc-specs > /dev/null 2>&1 || return 0
  echo "==> stopping the running instance"
  pkill -x pc-specs || true
  for _ in $(seq 1 50); do
    pgrep -x pc-specs > /dev/null 2>&1 || return 0
    sleep 0.1
  done
  pkill -9 -x pc-specs || true
  sleep 0.3
}

install_macos() {
  local dir="${dest:-/Applications}"
  if [ -z "$dest" ] && [ ! -w "$dir" ]; then
    dir="$HOME/Applications"
  fi
  mkdir -p "$dir"

  local stage="target/install/PC Specs.app"
  rm -rf "$stage"
  mkdir -p "$stage/Contents/MacOS" "$stage/Contents/Resources"
  cp "$bin" "$stage/Contents/MacOS/pc-specs"
  cp assets/icons/icon.icns "$stage/Contents/Resources/icon.icns"
  sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$stage/Contents/Info.plist"
  plutil -lint "$stage/Contents/Info.plist" > /dev/null
  # 没有开发者证书时做 ad-hoc 签名，保证 bundle 完整性校验通过。
  codesign --force --sign - "$stage"

  stop_running
  rm -rf "$dir/PC Specs.app"
  ditto "$stage" "$dir/PC Specs.app"
  echo "==> installed $dir/PC Specs.app"
  if [ "$launch" = 1 ]; then
    open "$dir/PC Specs.app"
  fi
}

install_linux() {
  local prefix="${dest:-$HOME/.local}"
  local id="com.xwzy.pc-specs"

  stop_running
  install -Dm755 "$bin" "$prefix/bin/pc-specs"
  # 桌面入口里的 Exec 换成绝对路径，不依赖 PATH 里有没有 ~/.local/bin。
  mkdir -p "$prefix/share/applications"
  sed "s|^Exec=.*|Exec=$prefix/bin/pc-specs|" "packaging/linux/$id.desktop" \
    > "$prefix/share/applications/$id.desktop"
  install -Dm644 assets/icons/32x32.png "$prefix/share/icons/hicolor/32x32/apps/$id.png"
  install -Dm644 assets/icons/128x128.png "$prefix/share/icons/hicolor/128x128/apps/$id.png"
  install -Dm644 "assets/icons/128x128@2x.png" "$prefix/share/icons/hicolor/256x256/apps/$id.png"
  if command -v update-desktop-database > /dev/null; then
    update-desktop-database "$prefix/share/applications" 2> /dev/null || true
  fi
  if command -v gtk-update-icon-cache > /dev/null; then
    gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" 2> /dev/null || true
  fi
  echo "==> installed $prefix/bin/pc-specs"
  if [ "$launch" = 1 ]; then
    nohup "$prefix/bin/pc-specs" > /dev/null 2>&1 &
  fi
}

case "$(uname -s)" in
  Darwin | Linux) ;;
  *)
    echo "unsupported system: $(uname -s)" >&2
    echo "Windows: cargo build --release --locked -p pc-specs, then run target/release/pc-specs.exe" >&2
    exit 2
    ;;
esac

echo "==> building PC Specs $version (release)"
cargo build --release --locked -p pc-specs

case "$(uname -s)" in
  Darwin) install_macos ;;
  Linux) install_linux ;;
esac
