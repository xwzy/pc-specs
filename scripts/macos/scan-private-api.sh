#!/usr/bin/env bash
# 静态扫描二进制里的「非公开 API」。上传 App Store Connect 时的校验会做类似的检查（ITMS-90338，
# 准则 2.5.1 只允许公开 API），提前自己查一遍，省得上传了才发现。
#
#   scripts/macos/scan-private-api.sh <二进制 | .app> [--arch arm64|x86_64]    默认扫描里面的每个架构
#
# 查三样：
#   1. 导入的符号（函数、常量、类）：不在 SDK 公开头文件里出现过的，多半是私有的。
#   2. 二进制里的字符串：长得像私有方法名（以下划线开头的 Objective-C 选择子），以及私有框架的路径。
#   3. 踩过的黑名单：下面的 DENY 里是已知的私有名字，不管上面两项有没有抓到，出现就是错。
#
# 有发现就以非 0 退出。结果是启发式的：没有发现不等于 Apple 一定放行，但发现的都应该处理掉。
# 第一次运行要把 SDK 的头文件整理成一份词表（20 秒左右），之后用 target/scan-cache/ 里的缓存。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

# 已知的私有名字（选择子、类名、符号）。出现在二进制的字符串或导入表里就算一处发现。
DENY=(
  # GPUI 的 macOS 平台层（商店版靠 packaging/macos/appstore/gpui-pre-macos.patch 去掉）
  _opaqueRectForWindowMoveWhenInTitlebar
  _windowRestorationOptions
  _zoomFill
  _windowResizeNorthWestSouthEastCursor
  _windowResizeNorthEastSouthWestCursor
  CAChameleonLayer
  # sysinfo 的 `component` 特性（读温度）：私有的 IOHID 事件系统
  IOHIDEventSystemClientCreate
  IOHIDEventSystemClientSetMatching
  IOHIDEventSystemClientCopyServices
  IOHIDServiceClientCopyEvent
  IOHIDServiceClientCopyProperty
  IOHIDEventGetFloatValue
  # 自己的传感器读取（SMC）：商店版不该有
  AppleSMC
)

target=""
only_arch=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      only_arch="${2:?--arch 需要参数：arm64 | x86_64}"
      shift
      ;;
    -h | --help)
      sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*) die "未知参数：${1}（试试 --help）" ;;
    *)
      [ -z "$target" ] || die "只能指定一个文件"
      target="$1"
      ;;
  esac
  shift
done
[ -n "$target" ] || die "需要指定要扫描的二进制或 .app（试试 --help）"
[ "$(uname -s)" = "Darwin" ] || die "只能在 macOS 上运行"
for tool in xcrun lipo strings awk comm; do need "$tool"; done

binary="$target"
if [ -d "$target" ]; then
  executable="$(plutil -extract CFBundleExecutable raw -o - "$target/Contents/Info.plist")"
  binary="$target/Contents/MacOS/${executable}"
fi
[ -f "$binary" ] || die "找不到二进制：${binary}"

# ---------------------------------------------------------------- SDK 公开头文件的词表

sdk_tokens() {
  local sdk version cache
  sdk="$(xcrun --sdk macosx --show-sdk-path)"
  version="$(xcrun --sdk macosx --show-sdk-version)"
  cache="$(target_dir)/scan-cache/sdk-tokens-${version}.txt"
  if [ ! -s "$cache" ]; then
    log "整理 macOS SDK ${version} 的公开头文件（第一次约 20 秒，之后用缓存）"
    mkdir -p "$(dirname "$cache")"
    find "${sdk}/System/Library/Frameworks" "${sdk}/usr/include" -name '*.h' -print0 \
      | xargs -0 cat \
      | LC_ALL=C tr -cs 'A-Za-z0-9_' '\n' \
      | LC_ALL=C sort -u > "${cache}.tmp"
    mv "${cache}.tmp" "$cache"
  fi
  printf '%s' "$cache"
}

tokens="$(sdk_tokens)"
work="$(mktemp -d "${TMPDIR:-/tmp}/pcs-scan.XXXXXX")"
trap 'rm -rf "$work"' EXIT
found=0

scan_arch() {
  local arch="$1" imports missing strings_file
  log "扫描 ${arch}：$(basename "$binary")"

  # 1. 导入的符号。去掉前导下划线、Objective-C 类 / 常量的前缀和 `$DARWIN_EXTSN` 之类的后缀，才能和头文件里的名字对上。
  #    dyld_info 的输出有两种：旧格式一行只有符号，链式修复（chained fixups）的新格式前面多一列偏移量。
  #    编译器自己插进去的运行时入口（ARC 的 objc_retain 一族、栈保护）没有公开头文件，不算。
  imports="${work}/imports-${arch}.txt"
  xcrun dyld_info -arch "$arch" -imports "$binary" 2> /dev/null \
    | awk '
      /^ +(0x[0-9A-Fa-f]+ +)?_/ {
        line = $0
        sub(/^ +(0x[0-9A-Fa-f]+ +)?/, "", line)
        sym = line
        sub(/ .*/, "", sym)
        lib = line
        sub(/^[^(]*\(from /, "", lib)
        sub(/\).*$/, "", lib)
        sub(/^_/, "", sym)
        sub(/^OBJC_(META)?CLASS_\$_/, "", sym)
        sub(/^OBJC_EHTYPE_\$_/, "", sym)
        sub(/\$.*$/, "", sym)
        if (lib == "libobjc" && sym ~ /^objc_/) next
        if (sym ~ /^(__stack_chk_fail|__stack_chk_guard|_tlv_bootstrap|dyld_stub_binder)$/) next
        print sym "\t" lib
      }' > "${imports}.raw"
  [ -s "${imports}.raw" ] || die "读不到 ${arch} 的导入表（dyld_info 的输出格式变了？）"
  cut -f 1 "${imports}.raw" | LC_ALL=C sort -u > "$imports"
  missing="${work}/missing-${arch}.txt"
  LC_ALL=C comm -23 "$imports" "$tokens" > "$missing"
  if [ -s "$missing" ]; then
    found=1
    warn "${arch}：下面这些导入的符号没有在 SDK 的公开头文件里出现过（多半是私有 API）："
    awk -F '\t' 'NR == FNR { miss[$1] = 1; next } ($1 in miss) { printf "    %s（来自 %s）\n", $1, $2 }' "$missing" "${imports}.raw" | sort -u >&2
  fi

  # 2. 字符串。objc2 的 sel!() 把选择子当普通 C 字符串存，所以要从字符串里找。
  strings_file="${work}/strings-${arch}.txt"
  strings -a -arch "$arch" -n 6 "$binary" > "$strings_file" 2> /dev/null || true
  local suspects="${work}/suspects-${arch}.txt"
  # 以下划线加小写字母开头、只含字母数字和冒号：私有 Objective-C 方法的惯用写法。
  grep -E '^_[a-z][A-Za-z0-9]{5,}(:[A-Za-z0-9]*)*:?$' "$strings_file" | LC_ALL=C sort -u > "$suspects" || true
  if [ -s "$suspects" ]; then
    found=1
    warn "${arch}：下面这些字符串长得像私有方法名（以下划线开头的选择子）："
    sed 's/^/    /' "$suspects" >&2
  fi
  if grep -q '/System/Library/PrivateFrameworks/' "$strings_file"; then
    found=1
    warn "${arch}：二进制里有私有框架的路径："
    grep '/System/Library/PrivateFrameworks/' "$strings_file" | LC_ALL=C sort -u | sed 's/^/    /' >&2
  fi

  # 3. 黑名单：在字符串里（去掉冒号后）或导入表里出现就算。
  local name hit=0
  for name in "${DENY[@]}"; do
    if grep -Eq "^_?${name}:?\$|[^A-Za-z0-9_]${name}[^A-Za-z0-9_]" "$strings_file" || grep -qx "$name" "$imports"; then
      warn "${arch}：出现了已知的私有名字 ${name}"
      hit=1
    fi
  done
  if [ "$hit" = 1 ]; then
    found=1
  fi

  if grep -qE '^(dlopen|dlsym)$' "$imports"; then
    log "${arch}：用到了 dlopen / dlsym（只是提示：确认加载的都是公开的库）"
  fi
}

for arch in $(lipo -archs "$binary"); do
  if [ -z "$only_arch" ] || [ "$only_arch" = "$arch" ]; then
    scan_arch "$arch"
  fi
done

if [ "$found" = 1 ]; then
  die "有疑似非公开 API 的发现，见上面的列表"
fi
log "没有发现非公开 API 的迹象"
