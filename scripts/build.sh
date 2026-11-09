#!/usr/bin/env bash
# 编译（可选再打包）发布版：按本机系统和目标平台，自动选原生编译还是 Docker。
#
#   scripts/build.sh [--platform macos|linux|windows] [--arch <架构>] [--package]
#     --platform  目标平台，默认当前系统
#     --arch      macOS：universal | arm64 | x64（默认 universal）；Linux、Windows：x64 | arm64（默认本机架构）
#     --package   编译完接着打发布用的安装包，放进 dist/
#
# 在哪台机器上能出什么：
#   目标 macOS    只能在 macOS 上（要 Xcode 的工具链和签名工具）。Intel 和 universal 在 Apple 芯片的 Mac 上也能编。
#   目标 Linux    本机是同架构的 Linux：原生编译。其他情况（macOS、别的架构）：用 Docker，总是编译并打包。
#   目标 Windows  只能在 Windows 上（GPUI 在 Windows 上要用 Windows SDK 里的 fxc.exe 编着色器，没法交叉编译）：
#                 直接用 PowerShell 脚本 scripts\windows\build.ps1、scripts\windows\package.ps1。
#   没有对应机器时，用 GitHub Actions 的 Release 工作流：它在各平台的原生 runner 上编译、签名、打包。
#
# 想分开控制（比如只打 deb，或只签名不公证），直接用各平台目录下的脚本：scripts/macos、scripts/linux、scripts/windows。
set -euo pipefail
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

platform=""
arch=""
package=0
while [ $# -gt 0 ]; do
  case "$1" in
    --platform)
      platform="${2:?--platform 需要参数：macos | linux | windows}"
      shift
      ;;
    --arch)
      arch="${2:?--arch 需要参数}"
      shift
      ;;
    --package) package=1 ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "未知参数：${1}（试试 --help）" ;;
  esac
  shift
done

case "$(uname -s)" in
  Darwin) host="macos" ;;
  Linux) host="linux" ;;
  *) die "Windows 上请直接用 PowerShell 脚本：scripts\\windows\\build.ps1 和 scripts\\windows\\package.ps1" ;;
esac
platform="${platform:-$host}"

# 要传给各平台脚本的 --arch；没指定就让它们用各自的默认值。
arch_args=()
if [ -n "$arch" ]; then
  arch_args=(--arch "$arch")
fi

case "$platform" in
  macos)
    [ "$host" = "macos" ] || die "macOS 版只能在 macOS 上编译；没有 Mac 的话，用 GitHub Actions 的 Release 工作流"
    "$ROOT/scripts/macos/build.sh" ${arch_args[@]+"${arch_args[@]}"}
    if [ "$package" = 1 ]; then
      "$ROOT/scripts/macos/package.sh" ${arch_args[@]+"${arch_args[@]}"}
    fi
    ;;

  linux)
    if [ "$host" = "linux" ] && { [ -z "$arch" ] || [ "$arch" = "$(host_arch)" ]; }; then
      "$ROOT/scripts/linux/build.sh" ${arch_args[@]+"${arch_args[@]}"}
      if [ "$package" = 1 ]; then
        "$ROOT/scripts/linux/package.sh" ${arch_args[@]+"${arch_args[@]}"}
      fi
    else
      if [ "$package" = 0 ]; then
        log "用 Docker 编译 Linux 版：产物都在容器里，所以总会同时打包（放进 dist/）"
      fi
      "$ROOT/scripts/linux/docker.sh" ${arch_args[@]+"${arch_args[@]}"}
    fi
    ;;

  windows)
    die "Windows 版只能在 Windows 上编译（GPUI 要用 Windows SDK 的 fxc.exe 编着色器，没法交叉编译）：用 scripts\\windows\\build.ps1，或者 GitHub Actions 的 Release 工作流"
    ;;

  *) die "不支持的平台：${platform}（macos | linux | windows）" ;;
esac
