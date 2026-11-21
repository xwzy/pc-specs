#!/usr/bin/env bash
# 编译「Mac App Store 版」：沙盒里审核能放行的部分，不含传感器、进程列表、系统清理、联网和本机接口服务。
#
#   scripts/build-appstore.sh [--arch universal|arm64|x64] [--package]
#     --package  接着签名并打成 .pkg，放进 dist/（要证书和描述文件，见 scripts/macos/appstore.sh --help）
#
# 其余操作用 scripts/macos/appstore.sh：check（自检）、package、validate（Apple 校验）、upload（上传）。
# 想在本机沙盒里试跑而不用证书：scripts/macos/appstore.sh package --adhoc。
set -euo pipefail
exec "$(dirname "$0")/build.sh" --edition appstore --platform macos "$@"
