#!/usr/bin/env bash
# 编译 macOS 发布版二进制（只能在 macOS 上运行：需要 Xcode 命令行工具里的 Metal 编译器）。
#
#   scripts/macos/build.sh [--arch universal|arm64|x64] [--edition full|appstore]
#     --arch     默认 universal
#     --edition  full：自行发布的全功能版（默认）
#                appstore：Mac App Store 版。沙盒里能放行的部分，最低 macOS 13，编译时给 GPUI 打补丁去掉私有 API，
#                          编完自动扫一遍非公开 API。细节见 docs/APP_STORE_COMPLIANCE.md
#
# 产物（都在 target/ 下；商店版换成 target/appstore/）：
#   arm64      target/aarch64-apple-darwin/release/pc-specs
#   x64        target/x86_64-apple-darwin/release/pc-specs
#   universal  上面两个用 lipo 合并成 target/universal-apple-darwin/release/pc-specs
# Apple 芯片的 Mac 可以直接交叉编译 x64，不需要 Intel 机器。
#
# 之后：全功能版用 scripts/macos/package.sh 打 .app / .dmg；商店版用 scripts/macos/appstore.sh 打 .pkg。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

arch="universal"
edition="full"
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：universal | arm64 | x64}"
      shift
      ;;
    --edition)
      edition="${2:?--edition 需要参数：full | appstore}"
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
check_edition "$edition"

# 不设的话，C / Objective-C 代码会按当前 SDK 的版本编译，产物在旧系统上起不来。
min_version="$(macos_min_version "$edition")"
export MACOSX_DEPLOYMENT_TARGET="$min_version"

# ---------------------------------------------------------------- 商店版：给 GPUI 打补丁

# GPUI 的 macOS 平台层里有几处私有 API（准则 2.5.1 只许公开 API），上游没有开关去掉它们。
# 所以编译商店版时，把补丁打在注册表里 gpui-pre-macos 的一份拷贝上，再用 cargo 的 [patch] 配置顶替原件。
# 补丁的来历和代价写在补丁文件的开头。拷贝只在补丁或 GPUI 版本变了才重做（动了文件 cargo 就要整个重编）。
gpui_patch="$ROOT/packaging/macos/appstore/gpui-pre-macos.patch"
patched_gpui() {
  local version dest stamp src dir
  version="$(awk '/^name = "gpui-pre-macos"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$ROOT/Cargo.lock")"
  [ -n "$version" ] || die "Cargo.lock 里没有 gpui-pre-macos"
  dest="$(target_dir)/appstore-patch/gpui-pre-macos-${version}"
  stamp="${version} $(shasum -a 256 "$gpui_patch" | cut -d ' ' -f 1)"
  if [ "$(cat "${dest}/.patch-stamp" 2> /dev/null || true)" != "$stamp" ]; then
    log "给 gpui-pre-macos ${version} 打补丁（商店版去掉私有 API）"
    src=""
    for dir in "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/gpui-pre-macos-"${version}"; do
      if [ -d "$dir" ]; then
        src="$dir"
        break
      fi
    done
    [ -n "$src" ] || die "找不到 gpui-pre-macos ${version} 的源码（先 cargo fetch --locked）"
    rm -rf "$dest"
    mkdir -p "$(dirname "$dest")"
    cp -R "$src" "$dest"
    (cd "$dest" && patch -p1 --forward --batch -s < "$gpui_patch") \
      || die "补丁打不上：GPUI 升级过吗？按 ${gpui_patch#"$ROOT"/} 开头的说明重新生成"
    printf '%s' "$stamp" > "${dest}/.patch-stamp"
  fi
  printf '%s' "$dest"
}

# [patch] 顶替了注册表里的原件，cargo 会顺手把锁文件里这个包的 source / checksum 两行去掉。
# 那不该留在仓库里：编完把锁文件原样放回去。
lock_backup=""
restore_lock() {
  if [ -n "$lock_backup" ] && [ -f "$lock_backup" ]; then
    cp "$lock_backup" "$ROOT/Cargo.lock"
    rm -f "$lock_backup"
  fi
}

cargo_flags=()
if [ "$edition" = "appstore" ]; then
  # 先确认锁文件和各 Cargo.toml 是一致的：打了补丁的编译不能带 --locked，这一步替它把关。
  (cd "$ROOT" && cargo fetch --locked > /dev/null)
  gpui_dir="$(patched_gpui)"
  lock_backup="$(mktemp "${TMPDIR:-/tmp}/pcs-cargo-lock.XXXXXX")"
  cp "$ROOT/Cargo.lock" "$lock_backup"
  trap restore_lock EXIT
  cargo_flags=(
    --no-default-features --features appstore
    --target-dir "$(edition_target_dir appstore)"
    --config "patch.crates-io.gpui-pre-macos.path=\"${gpui_dir}\""
  )
else
  cargo_flags=(--locked)
fi

build_one() {
  local triple="$1"
  if command -v rustup > /dev/null 2>&1; then
    rustup target add "$triple" > /dev/null
  fi
  log "cargo build --release --target ${triple}（${edition} 版，最低 macOS ${min_version}）"
  (cd "$ROOT" && cargo build --release -p "$PACKAGE" --target "$triple" "${cargo_flags[@]}")
}

# 在顶层直接调用才能校验 --arch：die 写在 $(...) 里只会退出子 shell，脚本会接着往下跑。
macos_triple "$arch" > /dev/null
result="$(macos_binary "$arch" "$edition")"

if [ "$arch" = "universal" ]; then
  build_one "$(macos_triple arm64)"
  build_one "$(macos_triple x64)"
  mkdir -p "$(dirname "$result")"
  lipo -create -output "$result" "$(macos_binary arm64 "$edition")" "$(macos_binary x64 "$edition")"
else
  build_one "$(macos_triple "$arch")"
fi

log "完成：${result}"
lipo -info "$result" >&2

if [ "$edition" = "appstore" ]; then
  "$ROOT/scripts/macos/scan-private-api.sh" "$result"
fi
