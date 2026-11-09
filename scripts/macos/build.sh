#!/usr/bin/env bash
# 编译 macOS 发布版二进制（只能在 macOS 上运行：需要 Xcode 命令行工具里的 Metal 编译器）。
#
#   scripts/macos/build.sh [--arch universal|arm64|x64]     默认 universal
#
# 产物（都在 target/ 下）：
#   arm64      target/aarch64-apple-darwin/release/pc-specs
#   x64        target/x86_64-apple-darwin/release/pc-specs
#   universal  上面两个用 lipo 合并成 target/universal-apple-darwin/release/pc-specs
# Apple 芯片的 Mac 可以直接交叉编译 x64，不需要 Intel 机器。
#
# 之后用 scripts/macos/package.sh 打 .app / .dmg。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

arch="universal"
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：universal | arm64 | x64}"
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

[ "$(uname -s)" = "Darwin" ] || die "macOS 版只能在 macOS 上编译；在别的系统上请用 CI（.github/workflows/release.yml）"
need cargo "https://rustup.rs"
need xcrun "xcode-select --install"

# 不设的话，C / Objective-C 代码会按当前 SDK 的版本编译，产物在旧系统上起不来。
export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN_VERSION"

build_one() {
  local triple="$1"
  if command -v rustup > /dev/null 2>&1; then
    rustup target add "$triple" > /dev/null
  fi
  log "cargo build --release --target ${triple}（最低 macOS ${MACOS_MIN_VERSION}）"
  (cd "$ROOT" && cargo build --release --locked -p "$PACKAGE" --target "$triple")
}

# 在顶层直接调用才能校验 --arch：die 写在 $(...) 里只会退出子 shell，脚本会接着往下跑。
macos_triple "$arch" > /dev/null
result="$(macos_binary "$arch")"

if [ "$arch" = "universal" ]; then
  build_one "$(macos_triple arm64)"
  build_one "$(macos_triple x64)"
  mkdir -p "$(dirname "$result")"
  lipo -create -output "$result" "$(macos_binary arm64)" "$(macos_binary x64)"
else
  build_one "$(macos_triple "$arch")"
fi

log "完成：${result}"
lipo -info "$result" >&2
