#!/usr/bin/env bash
# 给一个目录里的发布文件生成 SHA256SUMS.txt；设置了 GPG_PRIVATE_KEY 时，再生成分离签名
# SHA256SUMS.txt.asc 和公钥 pc-specs-signing-key.asc。
#
# macOS / Windows 的安装包有系统级的代码签名；Linux 的 deb / rpm / AppImage / tar.gz 没有，
# 用户用这份签过名的校验和来确认下载内容没被篡改：
#   gpg --import pc-specs-signing-key.asc
#   gpg --verify SHA256SUMS.txt.asc SHA256SUMS.txt && sha256sum -c SHA256SUMS.txt --ignore-missing
#
#   scripts/checksums.sh [目录]        默认 dist/
#
# 可选的环境变量：
#   GPG_PRIVATE_KEY   ASCII armor 格式的私钥（gpg --armor --export-secret-keys <id>）
#   GPG_PASSPHRASE    私钥口令（没有口令可以不设）
#   GPG_KEY_ID        私钥里有多个密钥时，指定用哪一个签名
set -euo pipefail
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"
# shellcheck source=lib/gpg.sh
source "$(dirname "$0")/lib/gpg.sh"

dir="${1:-$(dist_dir)}"
[ -d "$dir" ] || die "目录不存在：${dir}"
cd "$dir"

rm -f SHA256SUMS.txt SHA256SUMS.txt.asc pc-specs-signing-key.asc

sha256() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1"
  else
    shasum -a 256 "$1"
  fi
}

files="$(find . -maxdepth 1 -type f -print | sed 's|^\./||' | LC_ALL=C sort)"
[ -n "$files" ] || die "${dir} 里没有文件"
: > SHA256SUMS.txt
while IFS= read -r file; do
  sha256 "$file" >> SHA256SUMS.txt
done <<< "$files"
log "已生成 ${dir}/SHA256SUMS.txt（$(wc -l < SHA256SUMS.txt | tr -d ' ') 个文件）"

if ! gpg_available; then
  warn "没有设置 GPG_PRIVATE_KEY，校验和未签名"
  exit 0
fi

gpg_setup
trap gpg_teardown EXIT

gpg --batch --yes --quiet --pinentry-mode loopback --passphrase-file "$GPG_PASSFILE" \
  --local-user "$GPG_SIGN_KEY" --armor --detach-sign --output SHA256SUMS.txt.asc SHA256SUMS.txt
gpg --batch --armor --export "$GPG_SIGN_KEY" > pc-specs-signing-key.asc
gpg --batch --verify SHA256SUMS.txt.asc SHA256SUMS.txt 2> /dev/null || die "签名自检失败"
log "已用 GPG 签名：SHA256SUMS.txt.asc（公钥：pc-specs-signing-key.asc）"
