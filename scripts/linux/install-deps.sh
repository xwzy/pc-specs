#!/usr/bin/env bash
# 安装在 Linux 上编译（和打包）需要的系统依赖。支持 Debian / Ubuntu（apt）和 Fedora / RHEL（dnf）。
#
#   scripts/linux/install-deps.sh              只装编译依赖
#   scripts/linux/install-deps.sh --package    再装打 deb / rpm / AppImage 用的工具
#
# 不是 root 时自动用 sudo。运行时需要的 Vulkan 驱动、Wayland / X11 库由用户的系统提供，这里不装。
# 打 .deb 只能在 Debian / Ubuntu 上做（要用 dpkg-shlibdeps 算依赖）；.rpm 在两类系统上都可以。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

with_package=0
while [ $# -gt 0 ]; do
  case "$1" in
    --package) with_package=1 ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "未知参数：${1}（试试 --help）" ;;
  esac
  shift
done

[ "$(uname -s)" = "Linux" ] || die "这个脚本只在 Linux 上用"

sudo=""
if [ "$(id -u)" -ne 0 ]; then
  need sudo "或者用 root 运行"
  sudo="sudo"
fi

if command -v apt-get > /dev/null 2>&1; then
  packages=(
    build-essential clang pkg-config
    libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev
  )
  if [ "$with_package" = 1 ]; then
    packages+=(curl ca-certificates file binutils xz-utils gnupg dpkg-dev rpm desktop-file-utils appstream)
  fi
  log "apt 安装：${packages[*]}"
  $sudo apt-get update
  $sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${packages[@]}"
elif command -v dnf > /dev/null 2>&1; then
  packages=(
    gcc gcc-c++ make clang pkgconf-pkg-config
    fontconfig-devel wayland-devel libxkbcommon-x11-devel libX11-devel libxcb-devel openssl-devel libzstd-devel
  )
  if [ "$with_package" = 1 ]; then
    packages+=(curl file binutils xz gnupg2 rpm-build rpm-sign desktop-file-utils appstream)
  fi
  log "dnf 安装：${packages[*]}"
  $sudo dnf install -y "${packages[@]}"
else
  die "没有找到 apt-get 或 dnf：请参照这个脚本里的包名，用自己系统的包管理器装"
fi

if ! command -v cargo > /dev/null 2>&1; then
  warn "还没有 Rust 工具链：curl https://sh.rustup.rs | sh"
fi
