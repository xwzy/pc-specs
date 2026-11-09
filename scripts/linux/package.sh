#!/usr/bin/env bash
# Linux 发布打包：二进制 → tar.gz / deb / rpm / AppImage，放进 dist/。
#
#   scripts/linux/package.sh [选项]
#     --arch x64|arm64     默认本机架构，必须和二进制的架构一致
#     --binary <路径>      用指定的二进制，默认 target/release/pc-specs（scripts/linux/build.sh 的产物）
#     --formats <列表>     逗号分隔，默认 tar,deb,rpm,appimage
#
# 需要的工具用 scripts/linux/install-deps.sh --package 安装；打 .deb 只能在 Debian / Ubuntu 上做。
# 产物（<版本> 是根 Cargo.toml 里的版本号）：
#   pc-specs-<版本>-linux-<arch>.tar.gz            绿色版，解压即用
#   pc-specs_<版本>_<amd64|arm64>.deb              Debian / Ubuntu
#   pc-specs-<版本>-<release>.<x86_64|aarch64>.rpm Fedora / RHEL / openSUSE
#   pc-specs-<版本>-linux-<arch>.AppImage          免安装，各发行版通用
#
# 设置了 GPG_PRIVATE_KEY（见 scripts/lib/gpg.sh）时，会给 .rpm 签名。
# 所有文件的签名校验和用 scripts/checksums.sh 生成，deb / AppImage / tar.gz 靠它防篡改。
# SOURCE_DATE_EPOCH 决定包里的时间戳，默认取最近一次提交的时间：同样的源码和二进制打出同样的包。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"
# shellcheck source=../lib/gpg.sh
source "$(dirname "$0")/../lib/gpg.sh"
umask 022 # 目录和文件的权限不依赖运行环境的 umask

# AppImage 的打包工具。固定版本并校验 sha256：下载来的程序会在发布流程里执行，不能随便信。
APPIMAGETOOL_VERSION="1.9.1"
APPIMAGETOOL_SHA256_X64="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
APPIMAGETOOL_SHA256_ARM64="f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158"

arch=""
binary=""
formats="tar,deb,rpm,appimage"
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：x64 | arm64}"
      shift
      ;;
    --binary)
      binary="${2:?--binary 需要二进制的路径}"
      shift
      ;;
    --formats)
      formats="${2:?--formats 需要参数，如 tar,deb}"
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

[ "$(uname -s)" = "Linux" ] || die "不在 Linux 上：用 scripts/linux/docker.sh 在 Docker 里打包"
arch="${arch:-$(host_arch)}"
check_linux_arch "$arch"
[ -n "$binary" ] || binary="$(linux_binary)"
[ -f "$binary" ] || die "找不到二进制：${binary}（先运行 scripts/linux/build.sh）"

# 二进制的架构和 --arch 对不上时，打出来的包会装得上、跑不起来，这里提前拦住。
if command -v readelf > /dev/null 2>&1; then
  machine="$(readelf -h "$binary" | sed -n 's/^ *Machine: *//p')"
  case "${arch}:${machine}" in
    x64:*X86-64* | arm64:*AArch64*) ;;
    *) die "二进制的架构（${machine}）和 --arch ${arch} 不一致" ;;
  esac
fi

for format in $(printf '%s' "$formats" | tr ',' ' '); do
  case "$format" in
    tar | deb | rpm | appimage) ;;
    *) die "不支持的格式：${format}（tar | deb | rpm | appimage）" ;;
  esac
done
has_format() {
  case ",${formats}," in
    *",${1},"*) return 0 ;;
    *) return 1 ;;
  esac
}

version="$(app_version)"
dist="$(dist_dir)"
stage="$(target_dir)/package/linux-${arch}"
mkdir -p "$dist"
rm -rf "$stage"
mkdir -p "$stage"

epoch="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2> /dev/null || true)}"
epoch="${epoch:-$(date +%s)}"
export SOURCE_DATE_EPOCH="$epoch"

# deb 和 AppStream 的版本里，预发布后缀要写成 ~（1.2.3~rc.1 排在 1.2.3 前面），不能是 -。
deb_version="$(printf '%s' "$version" | sed 's/-/~/g')"

metainfo="${stage}/${APP_ID}.metainfo.xml"
sed -e "s/@VERSION@/${deb_version}/g" -e "s/@DATE@/$(date -u -d "@${epoch}" +%F)/g" \
  "$ROOT/packaging/linux/${APP_ID}.metainfo.xml" > "$metainfo"

# 有工具就顺手校验：元数据写坏了，软件中心里会显示不出来。
if command -v desktop-file-validate > /dev/null 2>&1; then
  desktop-file-validate "$ROOT/packaging/linux/${APP_ID}.desktop"
fi
if command -v appstreamcli > /dev/null 2>&1; then
  appstreamcli validate --no-net "$metainfo" > /dev/null
fi

# 桌面入口、图标、AppStream 元数据，按 FHS 布局装到 $1 下（deb 和 AppImage 共用）。
install_assets() {
  local root="$1"
  install -Dm644 "$ROOT/packaging/linux/${APP_ID}.desktop" "${root}/usr/share/applications/${APP_ID}.desktop"
  install -Dm644 "$ROOT/assets/icons/32x32.png" "${root}/usr/share/icons/hicolor/32x32/apps/${APP_ID}.png"
  install -Dm644 "$ROOT/assets/icons/128x128.png" "${root}/usr/share/icons/hicolor/128x128/apps/${APP_ID}.png"
  install -Dm644 "$ROOT/assets/icons/128x128@2x.png" "${root}/usr/share/icons/hicolor/256x256/apps/${APP_ID}.png"
  install -Dm644 "$metainfo" "${root}/usr/share/metainfo/${APP_ID}.metainfo.xml"
}

# ---------------------------------------------------------------- tar.gz

make_tar() {
  local name="pc-specs-${version}-linux-${arch}" dir
  dir="${stage}/${name}"
  install -Dm755 "$binary" "${dir}/pc-specs"
  install -Dm644 "$ROOT/packaging/linux/${APP_ID}.desktop" "${dir}/${APP_ID}.desktop"
  install -Dm644 "$ROOT/assets/icons/128x128@2x.png" "${dir}/${APP_ID}.png"
  install -Dm644 "$ROOT/LICENSE" "${dir}/LICENSE"
  tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@${epoch}" -C "$stage" -cf - "$name" \
    | gzip -n -9 > "${dist}/${name}.tar.gz"
  log "已生成 ${name}.tar.gz"
}

# ---------------------------------------------------------------- deb

# 运行时才 dlopen 的库，dpkg-shlibdeps 看不到，手写。要和 packaging/linux/pc-specs.spec 里的 Requires 保持一致。
DEB_RUNTIME_DEPENDS="libvulkan1 libxkbcommon-x11-0 libwayland-client0 libx11-xcb1 libfontconfig1"

make_deb() {
  need dpkg-deb "scripts/linux/install-deps.sh --package；打 .deb 只能在 Debian / Ubuntu 上做"
  need dpkg-shlibdeps "scripts/linux/install-deps.sh --package"
  local root="${stage}/deb-root" deb_arch shlibs depends pkg installed_size out
  case "$arch" in
    x64) deb_arch="amd64" ;;
    arm64) deb_arch="arm64" ;;
  esac

  install -Dm755 "$binary" "${root}/usr/bin/pc-specs"
  install_assets "$root"
  install -Dm644 "$ROOT/LICENSE" "${root}/usr/share/doc/pc-specs/copyright"

  # 链接进二进制的库交给 dpkg-shlibdeps：它会连 glibc 的最低版本一起算出来。
  # 它总是抱怨二进制不在 debian/<包名>/ 下面，这里不是在走 debian/rules，这条警告没有意义，滤掉；别的输出照常显示。
  mkdir -p "${stage}/shlibs/debian"
  printf 'Source: pc-specs\n\nPackage: pc-specs\nArchitecture: any\n' > "${stage}/shlibs/debian/control"
  shlibs="$(cd "${stage}/shlibs" && dpkg-shlibdeps -O -e"${root}/usr/bin/pc-specs" 2> "${stage}/shlibdeps.log" | sed -n 's/^shlibs:Depends=//p')"
  grep -v 'should already be installed in their package' "${stage}/shlibdeps.log" >&2 || true
  [ -n "$shlibs" ] || die "dpkg-shlibdeps 没有算出依赖"
  depends="$shlibs"
  for pkg in $DEB_RUNTIME_DEPENDS; do
    if ! printf '%s' "$shlibs" | tr ',' '\n' | sed 's/^ *//; s/[ (].*//' | grep -qx "$pkg"; then
      depends="${depends}, ${pkg}"
    fi
  done

  installed_size="$(du -sk "$root" | cut -f1)"
  mkdir -p "${root}/DEBIAN"
  cat > "${root}/DEBIAN/control" << EOF
Package: ${PACKAGE}
Version: ${deb_version}
Section: utils
Priority: optional
Architecture: ${deb_arch}
Maintainer: ${MAINTAINER}
Installed-Size: ${installed_size}
Depends: ${depends}
Recommends: mesa-vulkan-drivers
Homepage: ${HOMEPAGE}
Description: ${SUMMARY}
 Shows CPU, GPU, memory, storage, network and sensor details of this
 computer, with live charts, a tray icon and a local JSON API.
EOF
  out="${dist}/${PACKAGE}_${deb_version}_${deb_arch}.deb"
  # 用 xz 压缩：新版 dpkg 默认的 zstd 在老发行版上装不了。
  dpkg-deb --root-owner-group -Zxz --build "$root" "$out" > /dev/null
  log "已生成 $(basename "$out")（Depends: ${depends}）"
}

# ---------------------------------------------------------------- rpm

# Debian / Ubuntu 的 rpm 默认去找 /usr/bin/gpg2，那里没有，所以明确指定 PATH 里的 gpg。
sign_rpm() {
  rpmsign --addsign \
    --define "__gpg $(command -v gpg)" \
    --define "_gpg_path ${GNUPGHOME}" \
    --define "_gpg_name ${GPG_SIGN_KEY}" \
    --define "_gpg_digest_algo sha256" \
    --define "_gpg_sign_cmd_extra_args --batch --pinentry-mode loopback --passphrase-file ${GPG_PASSFILE}" \
    "$1" > /dev/null
}

make_rpm() {
  need rpmbuild "scripts/linux/install-deps.sh --package"
  local rpm_arch top="${stage}/rpmbuild" core pre release rpm
  case "$arch" in
    x64) rpm_arch="x86_64" ;;
    arm64) rpm_arch="aarch64" ;;
  esac
  # rpm 的 Version 里不能有 -，预发布版本要放进 Release，并且要排在正式版前面：
  #   1.2.3 → Version 1.2.3，Release 1；1.2.3-rc.1 → Version 1.2.3，Release 0.rc.1.1
  core="$(version_core "$version")"
  pre="${version#"$core"}"
  pre="${pre%%+*}"
  pre="${pre#-}"
  if [ -n "$pre" ]; then
    release="0.$(printf '%s' "$pre" | tr '-' '.').1"
  else
    release="1"
  fi

  mkdir -p "${top}/BUILD" "${top}/RPMS" "${top}/SOURCES" "${top}/SPECS" "${top}/SRPMS"
  rpmbuild -bb "$ROOT/packaging/linux/pc-specs.spec" --target "${rpm_arch}-linux" \
    --define "_topdir ${top}" \
    --define "pcs_version ${core}" \
    --define "pcs_release ${release}" \
    --define "pcs_root ${ROOT}" \
    --define "pcs_binary ${binary}" \
    --define "pcs_metainfo ${metainfo}" \
    --define "pcs_license ${ROOT}/LICENSE" \
    --define "_build_id_links none" \
    --define "_buildhost reproducible" \
    --define "clamp_mtime_to_source_date_epoch 1" \
    --define "use_source_date_epoch_as_buildtime 1" \
    --define "source_date_epoch_from_changelog 0" > "${stage}/rpmbuild.log" 2>&1 || {
    cat "${stage}/rpmbuild.log" >&2
    die "rpmbuild 失败"
  }
  rpm="$(find "${top}/RPMS" -name '*.rpm' -print -quit)"
  [ -n "$rpm" ] || die "rpmbuild 没有产出 .rpm"
  if gpg_available; then
    sign_rpm "$rpm"
    log "已签名 $(basename "$rpm")"
  else
    warn ".rpm 没有签名：设置 GPG_PRIVATE_KEY 后才会签名"
  fi
  cp "$rpm" "$dist/"
  log "已生成 $(basename "$rpm")"
}

# ---------------------------------------------------------------- AppImage

make_appimage() {
  need curl
  local tool_arch expected tool appdir out
  case "$arch" in
    x64)
      tool_arch="x86_64"
      expected="$APPIMAGETOOL_SHA256_X64"
      ;;
    arm64)
      tool_arch="aarch64"
      expected="$APPIMAGETOOL_SHA256_ARM64"
      ;;
  esac

  tool="$(target_dir)/tools/appimagetool-${APPIMAGETOOL_VERSION}-${tool_arch}.AppImage"
  if [ ! -x "$tool" ]; then
    mkdir -p "$(dirname "$tool")"
    log "下载 appimagetool ${APPIMAGETOOL_VERSION}（${tool_arch}）"
    curl -fsSL --retry 3 -o "${tool}.part" \
      "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-${tool_arch}.AppImage"
    printf '%s  %s\n' "$expected" "${tool}.part" | sha256sum -c - > /dev/null \
      || die "appimagetool 的 sha256 对不上，下载的文件不可信"
    chmod +x "${tool}.part"
    mv "${tool}.part" "$tool"
  fi

  appdir="${stage}/AppDir"
  install -Dm755 "$binary" "${appdir}/usr/bin/pc-specs"
  install_assets "$appdir"
  # AppImage 要求桌面入口、图标放在 AppDir 的顶层，AppRun 是启动入口。
  install -Dm644 "$ROOT/packaging/linux/${APP_ID}.desktop" "${appdir}/${APP_ID}.desktop"
  install -Dm644 "$ROOT/assets/icons/128x128@2x.png" "${appdir}/${APP_ID}.png"
  ln -s "${APP_ID}.png" "${appdir}/.DirIcon"
  ln -s usr/bin/pc-specs "${appdir}/AppRun"

  out="${dist}/pc-specs-${version}-linux-${arch}.AppImage"
  rm -f "$out"
  # --appimage-extract-and-run：appimagetool 自己也是 AppImage，这样在没有 FUSE 的 Docker / CI 里也能运行。
  ARCH="$tool_arch" "$tool" --appimage-extract-and-run "$appdir" "$out" > "${stage}/appimagetool.log" 2>&1 || {
    cat "${stage}/appimagetool.log" >&2
    die "appimagetool 失败"
  }
  chmod +x "$out"
  log "已生成 $(basename "$out")"
}

# ---------------------------------------------------------------- 主流程

if has_format rpm && gpg_available; then
  gpg_setup
  trap gpg_teardown EXIT
fi

log "打包 ${APP_NAME} ${version}（Linux ${arch}）：${formats}"
if has_format tar; then make_tar; fi
if has_format deb; then make_deb; fi
if has_format rpm; then make_rpm; fi
if has_format appimage; then make_appimage; fi

log "完成，文件在 ${dist}："
ls -lh "$dist" >&2
