#!/usr/bin/env bash
# macOS 发布打包：二进制 → PC Specs.app → 代码签名 → 公证 → .dmg（同样签名、公证、盖章）。
#
#   scripts/macos/package.sh [选项]
#     --arch universal|arm64|x64   默认 universal，对应 scripts/macos/build.sh 的产物
#     --binary <路径>              用指定的二进制，不按 --arch 去 target/ 下找
#     --notarize | --no-notarize   强制公证（缺凭据就报错）/ 只签名不公证；默认有凭据才公证
#     --app-only --out <目录>      只生成签好名的 PC Specs.app 放进目录，不打 dmg（本机安装用，
#                                  见 scripts/build-and-install.sh）
#
# 签名和公证全靠环境变量；一个都不设时只做 ad-hoc 签名：本机能跑，发给别人会被 Gatekeeper 拦下。
#   签名  APPLE_SIGNING_IDENTITY      钥匙串里的身份，如 "Developer ID Application: Name (TEAMID)"
#         APPLE_CERTIFICATE           .p12 证书的 base64（CI 用）。设置后导入一个临时钥匙串，用完即删；
#         APPLE_CERTIFICATE_PASSWORD  没设 APPLE_SIGNING_IDENTITY 时自动选其中的 Developer ID 身份
#   公证  下面三组凭据，按顺序取第一组齐全的：
#         APPLE_NOTARY_PROFILE        `xcrun notarytool store-credentials` 存下的 profile 名（本机推荐）
#         APPLE_API_KEY               App Store Connect API key 的 Key ID，
#         APPLE_API_ISSUER            Issuer ID，
#         APPLE_API_KEY_PATH          .p8 文件路径（或者用 APPLE_API_KEY_BASE64 传 .p8 内容的 base64）
#         APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID    Apple ID、App 专用密码、Team ID
#   有 Developer ID 签名、又有公证凭据时才会公证。
#
# 产物：dist/pc-specs-<版本>-macos-<架构>.dmg（目录可用 DIST_DIR 改）
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

arch="universal"
binary=""
notarize="auto"
app_only=0
out_dir=""
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：universal | arm64 | x64}"
      shift
      ;;
    --binary)
      binary="${2:?--binary 需要二进制的路径}"
      shift
      ;;
    --notarize) notarize="yes" ;;
    --no-notarize) notarize="no" ;;
    --app-only) app_only=1 ;;
    --out)
      out_dir="${2:?--out 需要目录}"
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

[ "$(uname -s)" = "Darwin" ] || die "macOS 版只能在 macOS 上打包；在别的系统上请用 CI（.github/workflows/release.yml）"
macos_triple "$arch" > /dev/null # 校验 --arch
for tool in codesign hdiutil plutil ditto security xcrun; do need "$tool"; done
if [ "$app_only" = 1 ] && [ -z "$out_dir" ]; then
  die "--app-only 需要同时指定 --out <目录>"
fi

version="$(app_version)"
[ -n "$binary" ] || binary="$(macos_binary "$arch")"
[ -f "$binary" ] || die "找不到二进制：${binary}（先运行 scripts/macos/build.sh --arch ${arch}）"

stage="$(target_dir)/package/macos-${arch}"
app="${stage}/${APP_NAME}.app"
work="$(mktemp -d "${TMPDIR:-/tmp}/pcs-macos.XXXXXX")"
keychain=""
saved_keychains=()

cleanup() {
  if [ -n "$keychain" ]; then
    security list-keychains -d user -s ${saved_keychains[@]+"${saved_keychains[@]}"} > /dev/null 2>&1 || true
    security delete-keychain "$keychain" > /dev/null 2>&1 || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# ---------------------------------------------------------------- 签名身份

# CI 里把 base64 的 .p12 导入一个临时钥匙串，并加进搜索列表，codesign 才找得到。
import_certificate() {
  [ -n "${APPLE_CERTIFICATE:-}" ] || return 0
  [ -n "${APPLE_CERTIFICATE_PASSWORD:-}" ] || die "设置了 APPLE_CERTIFICATE，还需要 APPLE_CERTIFICATE_PASSWORD"
  local password line cert="${work}/certificate.p12"
  password="$(uuidgen)"
  keychain="${work}/signing.keychain-db"
  printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$cert"

  # 记下原来的钥匙串搜索列表，结束时还原。每行形如    "/Users/x/Library/Keychains/login.keychain-db"
  while IFS= read -r line; do
    line="${line#*\"}"
    line="${line%\"*}"
    saved_keychains+=("$line")
  done < <(security list-keychains -d user)

  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  security import "$cert" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -f pkcs12 -A
  security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" > /dev/null
  security list-keychains -d user -s "$keychain" ${saved_keychains[@]+"${saved_keychains[@]}"}
  rm -f "$cert"
  log "证书已导入临时钥匙串"
}

identity="${APPLE_SIGNING_IDENTITY:-}"
identity_name="" # 给人看的证书名；identity 本身也可能是 SHA-1 指纹
developer_id=0

# 列出可用于代码签名的身份：用了临时钥匙串就只看它，否则看当前的搜索列表。
list_identities() {
  if [ -n "$keychain" ]; then
    security find-identity -p codesigning "$keychain"
  else
    security find-identity -p codesigning
  fi
}

choose_identity() {
  if [ -z "$identity" ] && [ -n "$keychain" ]; then
    # 优先对外分发用的 Developer ID Application，没有再取钥匙串里的第一个身份。
    # 输出每行形如   1) <SHA-1> "证书名"，用 awk 按引号切开取证书名（读完全部输入，避免 SIGPIPE）。
    identity="$(list_identities | awk -F'"' '/"Developer ID Application:/ && !found { name = $2; found = 1 } END { print name }')"
    if [ -z "$identity" ]; then
      identity="$(list_identities | awk -F'"' '/^ *1\) / { print $2 }')"
    fi
    [ -n "$identity" ] || die "导入的证书里没有可用于代码签名的身份"
  fi
  [ -n "$identity" ] || return 0

  identity_name="$identity"
  # APPLE_SIGNING_IDENTITY 也可以填 40 位的 SHA-1 指纹；要查出证书名，才知道是不是 Developer ID。
  if printf '%s' "$identity" | grep -Eq '^[0-9A-Fa-f]{40}$'; then
    identity_name="$(list_identities | awk -F'"' -v hash="$(printf '%s' "$identity" | tr 'a-f' 'A-F')" \
      'index($0, hash) && !found { name = $2; found = 1 } END { print name }')"
    [ -n "$identity_name" ] || die "钥匙串里找不到指纹为 ${identity} 的签名身份"
  fi
  case "$identity_name" in
    "Developer ID Application:"*) developer_id=1 ;;
  esac
}

# 有身份用身份签；没有身份退到 ad-hoc。额外参数原样交给 codesign。
# 时间戳要连 Apple 的服务器，公证必须有；只装到本机的 --app-only 不要，这样离线也能装。
sign() {
  local target="$1"
  shift
  if [ -z "$identity" ]; then
    codesign --force --sign - "$target"
    return
  fi
  local args=(--force --sign "$identity")
  if [ "$app_only" = 0 ]; then
    args+=(--timestamp)
  else
    args+=(--timestamp=none) # 明确不联网：Developer ID 签名默认会去取时间戳，离线时会失败
  fi
  if [ -n "$keychain" ]; then
    args+=(--keychain "$keychain")
  fi
  codesign "${args[@]}" "$@" "$target"
}

# ---------------------------------------------------------------- 公证

notary_args=()
choose_notary_credentials() {
  if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
    notary_args=(--keychain-profile "$APPLE_NOTARY_PROFILE")
  elif [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ]; then
    local key="${APPLE_API_KEY_PATH:-}"
    if [ -z "$key" ] && [ -n "${APPLE_API_KEY_BASE64:-}" ]; then
      key="${work}/AuthKey_${APPLE_API_KEY}.p8"
      printf '%s' "$APPLE_API_KEY_BASE64" | base64 --decode > "$key"
    fi
    [ -f "$key" ] || die "设置了 APPLE_API_KEY，还需要 APPLE_API_KEY_PATH（.p8 文件）或 APPLE_API_KEY_BASE64"
    notary_args=(--key "$key" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
  elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
    notary_args=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
  fi
}

# 提交 $1 去公证并等结果，通过后给 $2 盖章（把公证票据钉进文件，离线也能通过 Gatekeeper）。
notarize_and_staple() {
  local submit="$1" staple_target="$2" output status id
  log "提交公证：$(basename "$submit")（通常要几分钟）"
  output="$(xcrun notarytool submit "$submit" "${notary_args[@]}" --wait --timeout 30m --output-format json)" || {
    printf '%s\n' "$output" >&2
    die "公证提交失败"
  }
  status="$(printf '%s' "$output" | plutil -extract status raw -o - - 2> /dev/null || true)"
  id="$(printf '%s' "$output" | plutil -extract id raw -o - - 2> /dev/null || true)"
  if [ "$status" != "Accepted" ]; then
    xcrun notarytool log "$id" "${notary_args[@]}" >&2 || true
    die "公证没有通过（状态：${status:-未知}，id：${id:-未知}），原因见上面的日志"
  fi
  xcrun stapler staple "$staple_target"
}

# ---------------------------------------------------------------- 1. 组装 .app

log "组装 ${APP_NAME}.app（${version}，${arch}）"
rm -rf "$stage"
mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
install -m 0755 "$binary" "${app}/Contents/MacOS/${BIN_NAME}"
cp "$ROOT/assets/icons/icon.icns" "${app}/Contents/Resources/icon.icns"
sed -e "s/__VERSION__/$(version_core "$version")/g" \
  -e "s/__MIN_OS__/${MACOS_MIN_VERSION}/g" \
  -e "s/__YEAR__/$(date +%Y)/g" \
  "$ROOT/packaging/macos/Info.plist" > "${app}/Contents/Info.plist"
plutil -lint "${app}/Contents/Info.plist" > /dev/null

# ---------------------------------------------------------------- 2. 签名 .app

import_certificate
choose_identity

if [ -n "$identity" ]; then
  log "签名身份：${identity_name}"
  app_flags=(--options runtime) # 加固运行时：公证的硬性要求
  entitlements="${ROOT}/packaging/macos/entitlements.plist"
  if [ -f "$entitlements" ]; then
    app_flags+=(--entitlements "$entitlements")
  fi
  sign "$app" "${app_flags[@]}"
else
  warn "没有签名身份，只做 ad-hoc 签名：本机可以运行，分发给别人会被 Gatekeeper 拦下"
  sign "$app"
fi
codesign --verify --deep --strict "$app"

if [ "$app_only" = 1 ]; then
  rm -rf "${out_dir:?}/${APP_NAME}.app"
  mkdir -p "$out_dir"
  ditto "$app" "${out_dir}/${APP_NAME}.app"
  log "完成：${out_dir}/${APP_NAME}.app"
  exit 0
fi

# ---------------------------------------------------------------- 3. 公证 .app

do_notarize=0
if [ "$notarize" != "no" ]; then
  choose_notary_credentials
  if [ "${#notary_args[@]}" -gt 0 ] && [ "$developer_id" = 1 ]; then
    do_notarize=1
  elif [ "$notarize" = "yes" ]; then
    die "--notarize 需要 Developer ID Application 签名身份，以及公证凭据（见 --help）"
  else
    warn "跳过公证：需要 Developer ID Application 签名身份和公证凭据（见 --help）"
  fi
fi

if [ "$do_notarize" = 1 ]; then
  ditto -c -k --keepParent "$app" "${work}/app.zip"
  notarize_and_staple "${work}/app.zip" "$app"
fi

# ---------------------------------------------------------------- 4. 打 .dmg

dist="$(dist_dir)"
dmg="${dist}/pc-specs-${version}-macos-${arch}.dmg"
dmg_root="${stage}/dmg-root"
mkdir -p "$dist" "$dmg_root"
ditto "$app" "${dmg_root}/${APP_NAME}.app" # ditto 会保留已盖章的公证票据
ln -s /Applications "${dmg_root}/Applications"

log "创建 $(basename "$dmg")"
created=0
for attempt in 1 2 3; do
  rm -f "$dmg"
  if hdiutil create -volname "$APP_NAME" -srcfolder "$dmg_root" -ov -format UDZO -fs HFS+ "$dmg" > /dev/null; then
    created=1
    break
  fi
  warn "hdiutil 第 ${attempt} 次失败（CI 上偶尔会 Resource busy），5 秒后重试"
  sleep 5
done
[ "$created" = 1 ] || die "创建 dmg 失败"

if [ -n "$identity" ]; then
  sign "$dmg"
fi
if [ "$do_notarize" = 1 ]; then
  notarize_and_staple "$dmg" "$dmg"
fi

# ---------------------------------------------------------------- 5. 校验

hdiutil verify "$dmg" > /dev/null
if [ "$do_notarize" = 1 ]; then
  spctl --assess --type execute --verbose=2 "$app"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  xcrun stapler validate "$dmg"
fi

rm -rf "$dmg_root"
log "完成：${dmg}（$(du -h "$dmg" | cut -f1)）"
if [ "$do_notarize" = 1 ]; then
  log "已签名、公证并盖章"
elif [ -n "$identity" ]; then
  log "已签名，未公证"
else
  log "ad-hoc 签名，仅供本机使用"
fi
