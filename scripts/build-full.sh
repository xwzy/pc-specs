#!/usr/bin/env bash
# 编译「全功能版」（自行发布）：传感器、进程管理、系统清理、启动项、本机接口服务等功能全都在。
#
#   scripts/build-full.sh [--platform macos|linux|windows] [--arch <架构>] [--package]
#   参数同 scripts/build.sh（这个脚本只是把 --edition full 固定下来）。
#
# macOS 想直接发布：scripts/macos/package.sh 做签名 + 公证 + dmg；也可以走 GitHub Actions 的 Release 工作流。
set -euo pipefail
exec "$(dirname "$0")/build.sh" --edition full "$@"
