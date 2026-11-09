#!/usr/bin/env bash
# 一键编译并安装到本机（macOS / Linux）。
#
#   scripts/build-and-install.sh                      编译 release，装好并启动
#   scripts/build-and-install.sh --no-launch          只安装，不启动
#   scripts/build-and-install.sh --dir ~/Applications 装到别的目录（macOS 是 .app 所在目录，Linux 是前缀）
#
# macOS：装成 PC Specs.app，默认放 /Applications，没有写权限就退到 ~/Applications。
#        设了 APPLE_SIGNING_IDENTITY 就用这个证书签名（本机安装不联网、不公证），没设就 ad-hoc 签名。
# Linux：装到 ~/.local（二进制、桌面入口、图标），不需要 root。
# 正在运行的实例会先退出，装完再启动新的。
#
# 只编译本机架构；要出发布用的安装包，见 scripts/build.sh。
set -euo pipefail
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

launch=1
dest=""
while [ $# -gt 0 ]; do
  case "$1" in
    --no-launch) launch=0 ;;
    --dir)
      dest="${2:?--dir 需要一个目录}"
      shift
      ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "未知参数：${1}（试试 --help）" ;;
  esac
  shift
done

case "$(uname -s)" in
  Darwin | Linux) ;;
  *) die "不支持的系统：$(uname -s)。Windows 请用 scripts\\windows\\build.ps1 和 package.ps1" ;;
esac

version="$(app_version)"
bin="$(target_dir)/release/${BIN_NAME}"

# 先让正在跑的实例退出：单实例锁还在的话，新启动的会去叫醒旧的然后自己退出。
stop_running() {
  pgrep -x "$BIN_NAME" > /dev/null 2>&1 || return 0
  log "停止正在运行的实例"
  pkill -x "$BIN_NAME" || true
  for _ in $(seq 1 50); do
    pgrep -x "$BIN_NAME" > /dev/null 2>&1 || return 0
    sleep 0.1
  done
  pkill -9 -x "$BIN_NAME" || true
  sleep 0.3
}

install_macos() {
  local dir="${dest:-/Applications}" stage
  if [ -z "$dest" ] && [ ! -w "$dir" ]; then
    dir="$HOME/Applications"
  fi
  mkdir -p "$dir"

  stage="$(target_dir)/install"
  "$ROOT/scripts/macos/package.sh" --arch "$(host_arch)" --binary "$bin" --app-only --out "$stage"

  stop_running
  rm -rf "${dir:?}/${APP_NAME}.app"
  ditto "${stage}/${APP_NAME}.app" "${dir}/${APP_NAME}.app"
  log "已安装 ${dir}/${APP_NAME}.app"
  if [ "$launch" = 1 ]; then
    open "${dir}/${APP_NAME}.app"
  fi
}

install_linux() {
  local prefix="${dest:-$HOME/.local}"

  stop_running
  install -Dm755 "$bin" "${prefix}/bin/${BIN_NAME}"
  # 桌面入口里的 Exec 换成绝对路径，不依赖 PATH 里有没有 ~/.local/bin。
  mkdir -p "${prefix}/share/applications"
  sed "s|^Exec=.*|Exec=${prefix}/bin/${BIN_NAME}|" "$ROOT/packaging/linux/${APP_ID}.desktop" \
    > "${prefix}/share/applications/${APP_ID}.desktop"
  install -Dm644 "$ROOT/assets/icons/32x32.png" "${prefix}/share/icons/hicolor/32x32/apps/${APP_ID}.png"
  install -Dm644 "$ROOT/assets/icons/128x128.png" "${prefix}/share/icons/hicolor/128x128/apps/${APP_ID}.png"
  install -Dm644 "$ROOT/assets/icons/128x128@2x.png" "${prefix}/share/icons/hicolor/256x256/apps/${APP_ID}.png"
  if command -v update-desktop-database > /dev/null; then
    update-desktop-database "${prefix}/share/applications" 2> /dev/null || true
  fi
  if command -v gtk-update-icon-cache > /dev/null; then
    gtk-update-icon-cache -q -t "${prefix}/share/icons/hicolor" 2> /dev/null || true
  fi
  log "已安装 ${prefix}/bin/${BIN_NAME}"
  if [ "$launch" = 1 ]; then
    nohup "${prefix}/bin/${BIN_NAME}" > /dev/null 2>&1 &
  fi
}

log "编译 ${APP_NAME} ${version}（release）"
(cd "$ROOT" && cargo build --release --locked -p "$PACKAGE")

case "$(uname -s)" in
  Darwin) install_macos ;;
  Linux) install_linux ;;
esac
