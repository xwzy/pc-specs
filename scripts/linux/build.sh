#!/usr/bin/env bash
# 在 Linux 上编译发布版二进制（原生编译）。
#
#   scripts/linux/build.sh [--arch x64|arm64]     默认本机架构
#
# 产物：target/release/pc-specs，之后用 scripts/linux/package.sh 打包。
# 编译依赖用 scripts/linux/install-deps.sh 安装。想要的不是本机架构，或者不在 Linux 上，
# 用 scripts/linux/docker.sh（在 Docker 里编译）。
#
# 编译出的二进制要求 glibc 不低于编译机的版本：CI 在 Ubuntu 22.04 上编，所以最低是 glibc 2.35。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

arch=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：x64 | arm64}"
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

[ "$(uname -s)" = "Linux" ] || die "不在 Linux 上：用 scripts/linux/docker.sh 在 Docker 里编译"
need cargo "https://rustup.rs"
need pkg-config "先运行 scripts/linux/install-deps.sh"

arch="${arch:-$(host_arch)}"
check_linux_arch "$arch"
if [ "$arch" != "$(host_arch)" ]; then
  die "本机是 $(host_arch)，不能直接编译 ${arch}：用 scripts/linux/docker.sh --arch ${arch}，或者在 ${arch} 的机器上编译"
fi

log "cargo build --release（Linux ${arch}）"
(cd "$ROOT" && cargo build --release --locked -p "$PACKAGE")

result="$(linux_binary)"
log "完成：${result}"
