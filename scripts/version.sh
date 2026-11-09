#!/usr/bin/env bash
# 版本号管理。
#
# 整个项目只有一个版本来源：根 Cargo.toml 的 [workspace.package].version。各 crate 用
# `version.workspace = true` 继承；程序里的 env!("CARGO_PKG_VERSION")、macOS 的 Info.plist、
# Windows 安装包与文件属性、deb / rpm / AppImage、安装包文件名、发布用的 Git tag 全部由它派生，
# 任何地方都不要再手写版本号。
#
#   scripts/version.sh [get]                打印当前版本
#   scripts/version.sh set <版本>           修改版本（x.y.z 或 x.y.z-rc.1），并同步 Cargo.lock
#   scripts/version.sh bump <patch|minor|major>
#   scripts/version.sh tag                  打印发布用的 Git tag（v<版本>）
#   scripts/version.sh check                校验版本只有一个来源、Cargo.lock 已同步（发布流程会跑）
#
# 发版流程：scripts/version.sh set 0.3.0 → 提交并推送 → GitHub 上手动运行 Release 工作流。
set -euo pipefail
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

SEMVER='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'

set_version() {
  local new="$1" manifest="$ROOT/Cargo.toml" tmp
  [[ "$new" =~ $SEMVER ]] || die "不是合法的版本号：${new}（例：1.2.3 或 1.2.3-rc.1）"

  tmp="$(mktemp)"
  # 只改 [workspace.package] 段里的 version，别的段（依赖、profile）里同名的键不碰。
  awk -v ver="$new" '
    /^\[/ { section = $0 }
    section == "[workspace.package]" && /^version[[:space:]]*=/ && !done {
      print "version = \"" ver "\""
      done = 1
      next
    }
    { print }
    END { if (!done) exit 3 }
  ' "$manifest" > "$tmp" || {
    rm -f "$tmp"
    die "根 Cargo.toml 的 [workspace.package] 里没有找到 version"
  }
  cat "$tmp" > "$manifest"
  rm -f "$tmp"

  # Cargo.lock 里记着两个 workspace crate 的版本，必须一起更新，否则 `cargo build --locked` 会失败。
  # 优先离线（只改 workspace 自己的两行，不碰第三方依赖），离线不行再联网。
  (cd "$ROOT" && {
    cargo update --workspace --offline > /dev/null 2>&1 || cargo update --workspace > /dev/null
  })
  log "版本已改为 ${new}（Cargo.toml、Cargo.lock）"
}

bump_version() {
  local current core major minor patch
  current="$(app_version)"
  [[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] \
    || die "当前版本 ${current} 带有预发布后缀，bump 只处理 x.y.z；请用 set 指定版本"
  major="${BASH_REMATCH[1]}"
  minor="${BASH_REMATCH[2]}"
  patch="${BASH_REMATCH[3]}"
  case "$1" in
    major) core="$((major + 1)).0.0" ;;
    minor) core="$major.$((minor + 1)).0" ;;
    patch) core="$major.$minor.$((patch + 1))" ;;
    *) die "bump 的参数是 patch / minor / major" ;;
  esac
  set_version "$core"
}

check_version() {
  local version manifest bad=0
  version="$(app_version)"
  [[ "$version" =~ $SEMVER ]] || die "版本号不是合法的 SemVer：${version}"

  # 每个 crate 都必须继承 workspace 的版本，不允许自己再写一个。
  for manifest in "$ROOT"/crates/*/Cargo.toml; do
    if ! awk '
      /^\[/ { section = $0 }
      section == "[package]" && /^version\.workspace[[:space:]]*=[[:space:]]*true/ { found = 1 }
      END { exit !found }
    ' "$manifest"; then
      warn "${manifest#"$ROOT"/} 没有用 version.workspace = true 继承版本"
      bad=1
    fi
  done
  [ "$bad" = 0 ] || die "版本只能在根 Cargo.toml 的 [workspace.package] 里写一次"

  # Cargo.lock 必须和 Cargo.toml 一致（--locked：需要改锁文件就报错，不会偷偷改）。
  (cd "$ROOT" && cargo update --workspace --locked > /dev/null 2>&1) \
    || die "Cargo.lock 与 Cargo.toml 的版本不一致，运行 scripts/version.sh set ${version} 同步"

  log "版本 ${version}：唯一来源正确，Cargo.lock 已同步"
}

case "${1:-get}" in
  get) printf '%s\n' "$(app_version)" ;;
  set)
    [ $# -eq 2 ] || die "用法：scripts/version.sh set <版本>"
    set_version "$2"
    ;;
  bump)
    [ $# -eq 2 ] || die "用法：scripts/version.sh bump <patch|minor|major>"
    bump_version "$2"
    ;;
  tag) printf 'v%s\n' "$(app_version)" ;;
  check) check_version ;;
  -h | --help | help) sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//' ;;
  *) die "未知命令：$1（试试 --help）" ;;
esac
