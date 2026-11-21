#!/usr/bin/env bash
# Mac App Store 版：编译、打 .pkg、校验、上传。只能在 macOS 上运行。
#
#   scripts/macos/appstore.sh check                          自检：工具、证书、描述文件、API 密钥、协议，缺什么列出来
#   scripts/macos/appstore.sh build   [选项]                 只编译商店版（等同 scripts/macos/build.sh --edition appstore）
#   scripts/macos/appstore.sh package [选项]                 编译 → 组装 .app → 扫描私有 API → 签名 → .pkg → 自检 .pkg
#   scripts/macos/appstore.sh validate [选项]                打包后交给 Apple 校验（不上传）
#   scripts/macos/appstore.sh upload [选项]                  校验通过后上传到 App Store Connect
#
#   选项
#     --arch universal|arm64|x64   默认 universal
#     --skip-build                 不重新编译，用已有的商店版二进制（target/appstore/）
#     --adhoc                      （只用于 package）本机沙盒试跑：不需要证书和描述文件，用 ad-hoc 签名并带上沙盒权限，
#                                  换一个 Bundle ID（com.xwzy.pc-specs.sandbox-test），不生成 .pkg。
#                                  产物 dist/appstore-adhoc/PC Specs.app，双击就能运行；**不能**用来上架
#
# 凭据都走环境变量（不要写进脚本或仓库）：
#   APPLE_TEAM_ID                    开发者团队 ID，默认 W6684V32JU
#   APPSTORE_APP_IDENTITY            应用签名证书（默认自动找钥匙串里的 "Apple Distribution: … (团队 ID)"）
#   APPSTORE_INSTALLER_IDENTITY      安装包签名证书（默认自动找 "3rd Party Mac Developer Installer" / "Mac Installer Distribution"）
#   PROVISIONING_PROFILE_PATH        App Store 类型的描述文件，默认
#                                    ~/Library/Developer/Xcode/UserData/Provisioning Profiles/PCSpecs_Mac_App_Store.provisionprofile
#   APPLE_API_KEY / APPLE_API_ISSUER / APPLE_API_KEY_PATH    App Store Connect API 密钥（validate、upload 要用；
#                                    也可以用 APPLE_API_KEY_BASE64 传 .p8 内容的 base64）
#   BUILD_NUMBER                     临时覆盖构建号（默认根 Cargo.toml 的 [workspace.metadata.appstore].build，每次上传要比上次大）
#   SKIP_CHECK=1                     package / validate / upload 不先跑 check（不建议）
#
# 产物：dist/pc-specs-<版本>-b<构建号>-macos-appstore.pkg（目录可用 DIST_DIR 改）。第一次上架要做的事见 docs/RELEASE_CHECKLIST.md。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

usage() {
  sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

[ $# -gt 0 ] || {
  usage
  exit 1
}
command_name="$1"
shift
case "$command_name" in
  check | build | package | validate | upload) ;;
  -h | --help)
    usage
    exit 0
    ;;
  *) die "未知命令：${command_name}（check | build | package | validate | upload；试试 --help）" ;;
esac

arch="universal"
skip_build=0
adhoc=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：universal | arm64 | x64}"
      shift
      ;;
    --skip-build) skip_build=1 ;;
    --adhoc) adhoc=1 ;;
    -h | --help)
      usage
      exit 0
      ;;
    *) die "未知参数：${1}（试试 --help）" ;;
  esac
  shift
done
if [ "$adhoc" = 1 ] && [ "$command_name" != "package" ]; then
  die "--adhoc 只能和 package 一起用"
fi

[ "$(uname -s)" = "Darwin" ] || die "商店版只能在 macOS 上编译和打包"
macos_triple "$arch" > /dev/null # 校验 --arch

team_id="${APPLE_TEAM_ID:-W6684V32JU}"
profile="${PROVISIONING_PROFILE_PATH:-$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles/PCSpecs_Mac_App_Store.provisionprofile}"
work="$(mktemp -d "${TMPDIR:-/tmp}/pcs-appstore.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# ---------------------------------------------------------------- 钥匙串里的证书

# 在钥匙串里找名字以某个前缀开头、属于这个团队的证书，打印第一个（可以给多个前缀，按顺序试）。
# awk 读完全部输入再输出，避免提前退出时 security 收到 SIGPIPE。
find_identity() {
  local prefix name
  for prefix in "$@"; do
    name="$(security find-identity -v | awk -F '"' -v prefix="$prefix" -v team="(${team_id})" \
      'index($2, prefix) == 1 && index($2, team) && !found { name = $2; found = 1 } END { print name }')"
    if [ -n "$name" ]; then
      printf '%s' "$name"
      return 0
    fi
  done
}

app_identity="${APPSTORE_APP_IDENTITY:-}"
pkg_identity="${APPSTORE_INSTALLER_IDENTITY:-}"
find_identities() {
  if [ -z "$app_identity" ]; then
    app_identity="$(find_identity "Apple Distribution:" "3rd Party Mac Developer Application:")"
  fi
  if [ -z "$pkg_identity" ]; then
    pkg_identity="$(find_identity "3rd Party Mac Developer Installer:" "Mac Installer Distribution:")"
  fi
}

# ---------------------------------------------------------------- App Store Connect API 密钥

api_ready() {
  [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && { [ -f "${APPLE_API_KEY_PATH:-}" ] || [ -n "${APPLE_API_KEY_BASE64:-}" ]; }
}

# altool 只认「AuthKey_<密钥 ID>.p8」这个文件名，放在它搜索的目录里；用一个临时目录和 API_PRIVATE_KEYS_DIR 指给它。
prepare_api_key() {
  local dir="${work}/private_keys"
  mkdir -p "$dir"
  if [ -f "${APPLE_API_KEY_PATH:-}" ]; then
    ln -sf "$APPLE_API_KEY_PATH" "${dir}/AuthKey_${APPLE_API_KEY}.p8"
  else
    printf '%s' "$APPLE_API_KEY_BASE64" | base64 --decode > "${dir}/AuthKey_${APPLE_API_KEY}.p8"
  fi
  export API_PRIVATE_KEYS_DIR="$dir"
}

# ---------------------------------------------------------------- 描述文件

plist_get() {
  /usr/libexec/PlistBuddy -c "Print :$2" "$1" 2> /dev/null
}

decode_profile() {
  security cms -D -i "$profile" > "${work}/profile.plist" 2> /dev/null
}

# ---------------------------------------------------------------- check

problems=0
ok() { printf '  \033[32m✓\033[0m %s\n' "$*" >&2; }
bad() {
  printf '  \033[31m✗\033[0m %s\n' "$*" >&2
  problems=$((problems + 1))
}

# $1 = 1 时也检查 App Store Connect API 密钥（validate、upload 要用）。
run_checks() {
  local need_api="$1" tool expire now output
  problems=0
  log "自检（团队 ID ${team_id}，Bundle ID ${APP_ID}）"

  for tool in cargo xcrun codesign productbuild pkgutil plutil security patch shasum lipo; do
    if command -v "$tool" > /dev/null 2>&1; then :; else bad "缺少命令 ${tool}"; fi
  done
  if command -v rustup > /dev/null 2>&1 && ! rustup target list --installed 2> /dev/null | grep -q 'aarch64-apple-darwin'; then
    bad "rustup 没有装 aarch64-apple-darwin（rustup target add aarch64-apple-darwin x86_64-apple-darwin）"
  fi

  find_identities
  if [ -n "$app_identity" ]; then ok "应用签名证书：${app_identity}"; else bad "钥匙串里没有 \"Apple Distribution: … (${team_id})\"（developer.apple.com → Certificates 创建并装进钥匙串）"; fi
  if [ -n "$pkg_identity" ]; then ok "安装包签名证书：${pkg_identity}"; else bad "钥匙串里没有 \"3rd Party Mac Developer Installer\" / \"Mac Installer Distribution\" 证书（同上，创建后装进钥匙串）"; fi

  if [ ! -f "$profile" ]; then
    bad "找不到描述文件：${profile}（developer.apple.com：先注册 App ID ${APP_ID}，再创建 Mac App Store Connect 类型的描述文件，下载到这里；或设置 PROVISIONING_PROFILE_PATH）"
  elif ! decode_profile; then
    bad "描述文件读不出来：${profile}"
  else
    if [ "$(plist_get "${work}/profile.plist" 'Entitlements:com.apple.application-identifier')" = "${team_id}.${APP_ID}" ]; then
      ok "描述文件：App ID ${team_id}.${APP_ID}"
    else
      bad "描述文件的 App ID 不是 ${team_id}.${APP_ID}（通配符的不行，要为这个 Bundle ID 单独建）"
    fi
    if plist_get "${work}/profile.plist" 'ProvisionedDevices' > /dev/null; then
      bad "这是开发用的描述文件（限定了设备）：要的是 Mac App Store Connect 类型"
    fi
    if plist_get "${work}/profile.plist" 'Platform' | grep -q 'OSX'; then :; else bad "描述文件不是 macOS 平台的"; fi
    expire="$(plutil -extract ExpirationDate raw -o - "${work}/profile.plist" 2> /dev/null || true)"
    now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    if [ -n "$expire" ] && [[ "$expire" > "$now" ]]; then ok "描述文件有效期到 ${expire}"; else bad "描述文件已过期（${expire:-未知}）：重新生成并下载"; fi
  fi

  if [ "$need_api" = 1 ]; then
    if api_ready; then
      prepare_api_key
      if output="$(xcrun notarytool history --key "${API_PRIVATE_KEYS_DIR}/AuthKey_${APPLE_API_KEY}.p8" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" 2>&1)"; then
        ok "App Store Connect API 密钥可用，协议有效"
      elif printf '%s' "$output" | grep -qi 'agreement'; then
        bad "Apple 开发者协议还没签或已过期：登录 developer.apple.com / App Store Connect 接受最新协议"
      else
        bad "API 密钥不可用：$(printf '%s' "$output" | tail -n 1)"
      fi
    else
      bad "没有设置 App Store Connect API 密钥：APPLE_API_KEY、APPLE_API_ISSUER、APPLE_API_KEY_PATH（用户和访问 → 集成 → App Store Connect API 里创建）"
    fi
  fi
  return 0
}

require_checks() {
  if [ "${SKIP_CHECK:-0}" = 1 ]; then
    find_identities
    return 0
  fi
  run_checks "$1"
  [ "$problems" = 0 ] || die "自检没通过（${problems} 项），先处理上面标 ✗ 的"
}

if [ "$command_name" = "check" ]; then
  run_checks 1
  if [ "$problems" = 0 ]; then
    log "全部就绪"
  else
    die "还有 ${problems} 项没准备好（上面标 ✗ 的）。另外别忘了在 App Store Connect 里先创建 ${APP_ID} 这个 App 的记录"
  fi
  exit 0
fi

# ---------------------------------------------------------------- 编译

build_binary() {
  if [ "$skip_build" = 0 ]; then
    "$ROOT/scripts/macos/build.sh" --edition appstore --arch "$arch"
  fi
  [ -f "$(macos_binary "$arch" appstore)" ] || die "找不到商店版二进制：$(macos_binary "$arch" appstore)（去掉 --skip-build 重新编译）"
}

if [ "$command_name" = "build" ]; then
  skip_build=0
  build_binary
  exit 0
fi

# ---------------------------------------------------------------- 组装 .app

version="$(app_version)"
build="$(app_build_number)"
min_os="$(macos_min_version appstore)"
binary="$(macos_binary "$arch" appstore)"
stage="$(target_dir)/package/appstore-${arch}"
app="${stage}/${APP_NAME}.app"
resources="$ROOT/packaging/macos/appstore"

# Xcode 编译时会写进 Info.plist 的编译环境信息（App Store Connect 用它们判断用什么工具链编的）。
add_build_environment() {
  local plist="$1" sdk_version sdk_build xcode_version xcode_build xcode_code
  sdk_version="$(xcrun --sdk macosx --show-sdk-version 2> /dev/null || true)"
  sdk_build="$(xcrun --sdk macosx --show-sdk-build-version 2> /dev/null || true)"
  xcode_version="$(xcodebuild -version 2> /dev/null | awk '/^Xcode/ { print $2 }' || true)"
  xcode_build="$(xcodebuild -version 2> /dev/null | awk '/^Build version/ { print $3 }' || true)"
  if [ -z "$sdk_version" ] || [ -z "$sdk_build" ] || [ -z "$xcode_version" ] || [ -z "$xcode_build" ]; then
    warn "读不到 Xcode / SDK 的版本，Info.plist 里不写 DT* 编译环境信息（装完整的 Xcode 再打包）"
    return 0
  fi
  # Xcode 26.5 → 2650，16.2.1 → 1621：两位主版本 + 一位次版本 + 一位修订号。
  xcode_code="$(printf '%s' "$xcode_version" | awk -F '.' '{ printf "%02d%d%d", $1, $2, $3 }')"
  local entry
  for entry in \
    "BuildMachineOSBuild:$(sw_vers -buildVersion)" \
    "DTCompiler:com.apple.compilers.llvm.clang.1_0" \
    "DTPlatformBuild:${sdk_build}" \
    "DTPlatformName:macosx" \
    "DTPlatformVersion:${sdk_version}" \
    "DTSDKBuild:${sdk_build}" \
    "DTSDKName:macosx${sdk_version}" \
    "DTXcode:${xcode_code}" \
    "DTXcodeBuild:${xcode_build}"; do
    /usr/libexec/PlistBuddy -c "Add :${entry%%:*} string ${entry#*:}" "$plist"
  done
}

assemble_app() {
  log "组装 ${APP_NAME}.app（${version}，构建号 ${build}，${arch}，最低 macOS ${min_os}）"
  rm -rf "$stage"
  mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
  install -m 0755 "$binary" "${app}/Contents/MacOS/${BIN_NAME}"
  install -m 0644 "$ROOT/assets/icons/icon.icns" "${app}/Contents/Resources/icon.icns"
  install -m 0644 "${resources}/PrivacyInfo.xcprivacy" "${app}/Contents/Resources/PrivacyInfo.xcprivacy"
  sed -e "s/__VERSION__/$(version_core "$version")/g" \
    -e "s/__BUILD__/${build}/g" \
    -e "s/__MIN_OS__/${min_os}/g" \
    -e "s/__YEAR__/$(date +%Y)/g" \
    "${resources}/Info.plist" > "${app}/Contents/Info.plist"
  add_build_environment "${app}/Contents/Info.plist"
  plutil -lint "${app}/Contents/Info.plist" > /dev/null
  plutil -lint "${app}/Contents/Resources/PrivacyInfo.xcprivacy" > /dev/null
}

# 签名时要带的权限：我们自己的沙盒权限，加上描述文件里的 com.apple.application-identifier 等。
merged_entitlements() {
  local merged="${work}/entitlements.plist"
  cp "${resources}/entitlements.plist" "$merged"
  /usr/libexec/PlistBuddy -x -c 'Print :Entitlements' "${work}/profile.plist" > "${work}/profile-entitlements.plist"
  /usr/libexec/PlistBuddy -c "Merge ${work}/profile-entitlements.plist" "$merged" 2> /dev/null || true
  plutil -lint "$merged" > /dev/null
  [ "$(plist_get "$merged" 'com.apple.application-identifier')" = "${team_id}.${APP_ID}" ] \
    || die "合并后的权限里没有 com.apple.application-identifier = ${team_id}.${APP_ID}（描述文件对吗？）"
  printf '%s' "$merged"
}

# ---------------------------------------------------------------- 打包后的检查

# 把 .pkg 展开，逐项确认 Apple 会查的东西：签名、描述文件、权限、隐私清单、文件权限。
verify_pkg() {
  local pkg_file="$1" expanded="${work}/expanded" inner entitlements
  pkgutil --check-signature "$pkg_file" > "${work}/pkg-signature.txt" 2>&1 || die "安装包签名无效：$(cat "${work}/pkg-signature.txt")"
  grep -Eq 'Installer' "${work}/pkg-signature.txt" || die "安装包没有用 Installer 证书签名（见 pkgutil --check-signature）"
  pkgutil --expand-full "$pkg_file" "$expanded"
  inner="$(find "$expanded" -maxdepth 4 -type d -name "${APP_NAME}.app" | head -n 1)"
  [ -n "$inner" ] || die "安装包里没有 ${APP_NAME}.app"
  codesign --verify --deep --strict "$inner" || die "安装包里的 .app 签名校验没通过"
  [ -f "${inner}/Contents/embedded.provisionprofile" ] || die "安装包里的 .app 没有内嵌描述文件"
  [ -f "${inner}/Contents/Resources/PrivacyInfo.xcprivacy" ] || die "安装包里的 .app 缺隐私清单 PrivacyInfo.xcprivacy"
  entitlements="$(codesign -d --entitlements :- "$inner" 2> /dev/null || true)"
  printf '%s' "$entitlements" | grep -q 'com.apple.security.app-sandbox' || die "签名里没有沙盒权限（app-sandbox）"
  printf '%s' "$entitlements" | grep -q "${team_id}.${APP_ID}" || die "签名里的 application-identifier 不是 ${team_id}.${APP_ID}"
  if [ -n "$(find "$inner" ! -perm -o=r -print -quit)" ]; then
    die "安装包里有文件对其他用户不可读（上传时会被拒）：$(find "$inner" ! -perm -o=r -print | head -n 3 | tr '\n' ' ')"
  fi
  log "安装包自检通过"
}

# ---------------------------------------------------------------- package

# 交给 altool：1 = 只校验，2 = 校验后上传。
altool_run() {
  local action="$1" pkg_file="$2"
  api_ready || die "需要 App Store Connect API 密钥：APPLE_API_KEY、APPLE_API_ISSUER、APPLE_API_KEY_PATH"
  prepare_api_key
  log "$([ "$action" = validate ] && echo '提交 Apple 校验' || echo '上传到 App Store Connect')：$(basename "$pkg_file")"
  if [ "$action" = validate ]; then
    xcrun altool --validate-app -f "$pkg_file" -t macos --api-key "$APPLE_API_KEY" --api-issuer "$APPLE_API_ISSUER"
  else
    xcrun altool --upload-app -f "$pkg_file" -t macos --api-key "$APPLE_API_KEY" --api-issuer "$APPLE_API_ISSUER"
  fi
}

if [ "$adhoc" = 1 ]; then
  build_binary
  assemble_app
  # 换个 Bundle ID，不和装在 /Applications 里的正式版（同一个 ID）抢；沙盒容器也是分开的。
  /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier ${APP_ID}.sandbox-test" "${app}/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleName ${APP_NAME} (sandbox test)" "${app}/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName ${APP_NAME} (sandbox test)" "${app}/Contents/Info.plist"
  "$ROOT/scripts/macos/scan-private-api.sh" "$app"
  # 和正式签名一样开着强化运行时，试跑才和上架后的行为一致（只是证书换成 ad-hoc）。
  codesign --force --sign - --options runtime --entitlements "${resources}/entitlements.plist" "$app"
  codesign --verify --deep --strict "$app"
  out="$(dist_dir)/appstore-adhoc"
  rm -rf "${out:?}/${APP_NAME}.app"
  mkdir -p "$out"
  ditto "$app" "${out}/${APP_NAME}.app"
  log "完成：${out}/${APP_NAME}.app（ad-hoc 签名、带沙盒权限，只用来在本机试跑，不能上架）"
  exit 0
fi

require_checks "$([ "$command_name" = package ] && echo 0 || echo 1)"
find_identities
decode_profile || die "描述文件读不出来：${profile}"

build_binary
assemble_app
"$ROOT/scripts/macos/scan-private-api.sh" "$app"

log "签名 .app：${app_identity}"
cp "$profile" "${app}/Contents/embedded.provisionprofile"
chmod 0644 "${app}/Contents/embedded.provisionprofile"
entitlements="$(merged_entitlements)"
codesign --force --sign "$app_identity" --options runtime --timestamp --entitlements "$entitlements" "$app"
codesign --verify --deep --strict "$app"

pkg="$(dist_dir)/pc-specs-${version}-b${build}-macos-appstore.pkg"
mkdir -p "$(dist_dir)"
rm -f "$pkg"
log "生成安装包：${pkg}"
productbuild --component "$app" /Applications --sign "$pkg_identity" "$pkg" > /dev/null
verify_pkg "$pkg"

if [ "$command_name" = "validate" ] || [ "$command_name" = "upload" ]; then
  altool_run validate "$pkg"
fi
if [ "$command_name" = "upload" ]; then
  altool_run upload "$pkg"
  log "已上传。到 App Store Connect 的 TestFlight / 构建版本里等处理完成（通常几分钟到半小时），再选择构建版本提交审核"
fi

log "完成：${pkg}"
shasum -a 256 "$pkg" >&2
