#!/usr/bin/env bash
# 用 Docker 编译并打包 Linux 版：在 macOS、Windows（Docker Desktop / WSL）或别的架构的机器上也能出 Linux 包。
#
#   scripts/linux/docker.sh [选项]
#     --arch x64|arm64     默认本机架构
#     --formats <列表>     同 scripts/linux/package.sh，默认 tar,deb,rpm,appimage
#
# 产物和原生打包一样放在 dist/。镜像是 Ubuntu 22.04（见 scripts/linux/Dockerfile）。
# 要的架构和本机不同时，Docker 会模拟运行，编译会慢很多（Apple 芯片上的 OrbStack / Docker Desktop 跑 x64 用 Rosetta）。
# cargo 的下载缓存和 target/ 放在 Docker volume（pc-specs-cache-<arch>）里：不弄脏仓库，重复构建是增量的。
# 清理缓存：docker volume rm pc-specs-cache-x64 pc-specs-cache-arm64
#
# 设置了 GPG_PRIVATE_KEY / GPG_PASSPHRASE / GPG_KEY_ID 会传进容器，用来给 .rpm 签名。
set -euo pipefail
# shellcheck source=../lib/common.sh
source "$(dirname "$0")/../lib/common.sh"

arch=""
formats="tar,deb,rpm,appimage"
while [ $# -gt 0 ]; do
  case "$1" in
    --arch)
      arch="${2:?--arch 需要参数：x64 | arm64}"
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

need docker "https://docs.docker.com/get-docker/"
arch="${arch:-$(host_arch)}"
check_linux_arch "$arch"
case "$arch" in
  x64) platform="linux/amd64" ;;
  arm64) platform="linux/arm64" ;;
esac
if [ "$arch" != "$(host_arch)" ]; then
  warn "本机是 $(host_arch)，${arch} 要靠模拟运行，会很慢"
fi

image="pc-specs-linux-build:${arch}"
log "构建镜像 ${image}（${platform}）"
docker build --platform "$platform" -t "$image" -f "$ROOT/scripts/linux/Dockerfile" "$ROOT/scripts"

dist="$(dist_dir)"
mkdir -p "$dist"
epoch="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2> /dev/null || date +%s)}"

log "在容器里编译并打包（${arch}：${formats}）"
# 源码只读挂进去；容器以 root 运行，结束时把 dist/ 里的文件还给当前用户。
docker run --rm --platform "$platform" \
  -v "${ROOT}:/work:ro" \
  -v "${dist}:/dist" \
  -v "pc-specs-cache-${arch}:/cache" \
  -w /work \
  -e CARGO_HOME=/cache/cargo \
  -e CARGO_TARGET_DIR=/cache/target \
  -e DIST_DIR=/dist \
  -e SOURCE_DATE_EPOCH="$epoch" \
  -e HOST_UID="$(id -u)" \
  -e HOST_GID="$(id -g)" \
  -e GPG_PRIVATE_KEY \
  -e GPG_PASSPHRASE \
  -e GPG_KEY_ID \
  "$image" bash -euo pipefail -c '
    trap "chown -R ${HOST_UID}:${HOST_GID} /dist" EXIT
    scripts/linux/build.sh --arch "$0"
    scripts/linux/package.sh --arch "$0" --formats "$1"
  ' "$arch" "$formats"

log "完成，文件在 ${dist}"
