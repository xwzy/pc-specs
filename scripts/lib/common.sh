# shellcheck shell=bash
# shellcheck disable=SC2034 # 这里的常量是给 source 它的脚本用的
# macOS / Linux 各脚本共用的常量与函数，用 `source` 引入，不直接执行。
# （Windows 的 PowerShell 脚本是独立的，同样的常量在 scripts/windows/*.ps1 的开头。）
#
# 写法兼容 macOS 自带的 bash 3.2：不用关联数组、mapfile、${var^^}，空数组展开要写成 ${a[@]+"${a[@]}"}。

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

APP_NAME="PC Specs"               # 给人看的名字：.app 名、安装包标题
APP_ID="com.xwzy.pc-specs"        # 反向域名标识：Bundle ID、.desktop / AppStream ID
BIN_NAME="pc-specs"               # 可执行文件名
PACKAGE="pc-specs"                # cargo 包名
HOMEPAGE="https://github.com/xwzy/pc-specs"
MAINTAINER="pc-specs contributors <noreply@users.noreply.github.com>"
SUMMARY="PC configuration viewer and live monitor"

# macOS 最低系统版本：编译时的 deployment target 与 Info.plist 的 LSMinimumSystemVersion 共用这一个值。
MACOS_MIN_VERSION="${MACOS_MIN_VERSION:-11.0}"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" > /dev/null 2>&1 || die "缺少命令 ${1}${2:+（${2}）}"
}

# 版本号的唯一来源是根 Cargo.toml 的 [workspace.package].version，直接读这个文件。
# 不用 `cargo pkgid`：Cargo.lock 过期时它报的是锁文件里的旧版本，会让安装包悄悄带上错的版本号。
# 锁文件有没有跟上由 `scripts/version.sh check` 和 `cargo build --locked` 负责拦住。
app_version() {
  local version
  version="$(awk '
    /^\[/ { section = $0 }
    section == "[workspace.package]" && /^version[[:space:]]*=/ {
      line = $0
      sub(/^version[[:space:]]*=[[:space:]]*"/, "", line)
      sub(/".*$/, "", line)
      print line
      exit
    }
  ' "$ROOT/Cargo.toml")"
  [ -n "$version" ] || die "根 Cargo.toml 的 [workspace.package] 里没有找到 version"
  printf '%s' "$version"
}

# 去掉预发布 / 构建元数据后缀：1.2.3-rc.1+abc → 1.2.3。macOS 的 Bundle 版本、Windows 的文件版本只认纯数字。
version_core() {
  printf '%s' "${1%%[-+]*}"
}

# cargo 的输出目录（尊重 CARGO_TARGET_DIR，Docker 构建会用到）。
target_dir() {
  printf '%s' "${CARGO_TARGET_DIR:-$ROOT/target}"
}

# 最终发布文件的输出目录。
dist_dir() {
  printf '%s' "${DIST_DIR:-$ROOT/dist}"
}

# 本机架构，用统一的名字：x64 / arm64。
host_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) printf 'x64' ;;
    arm64 | aarch64) printf 'arm64' ;;
    *) die "不支持的 CPU 架构：$(uname -m)" ;;
  esac
}

# macOS：统一的架构名 → cargo 的 target 目录名。universal 不是真正的 target，是 lipo 合并后的产物目录。
macos_triple() {
  case "$1" in
    arm64) printf 'aarch64-apple-darwin' ;;
    x64) printf 'x86_64-apple-darwin' ;;
    universal) printf 'universal-apple-darwin' ;;
    *) die "不支持的 macOS 架构：${1}（universal | arm64 | x64）" ;;
  esac
}

# 某个架构编译出来的 macOS 二进制放在哪。
macos_binary() {
  printf '%s/%s/release/%s' "$(target_dir)" "$(macos_triple "$1")" "$BIN_NAME"
}

# Linux 只做本机原生编译（别的架构用 scripts/linux/docker.sh，或 CI 里对应架构的 runner），产物在 target/release/。
linux_binary() {
  printf '%s/release/%s' "$(target_dir)" "$BIN_NAME"
}

check_linux_arch() {
  case "$1" in
    x64 | arm64) ;;
    *) die "不支持的 Linux 架构：${1}（x64 | arm64）" ;;
  esac
}
