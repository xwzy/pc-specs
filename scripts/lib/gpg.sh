# shellcheck shell=bash
# GPG 签名的公共部分，用 `source` 引入（先 source common.sh）。
#
# 用到的环境变量：
#   GPG_PRIVATE_KEY   ASCII armor 格式的私钥（gpg --armor --export-secret-keys <id>）
#   GPG_PASSPHRASE    私钥口令（没有口令可以不设）
#   GPG_KEY_ID        私钥里有多个密钥时，指定用哪一个签名
#
#   gpg_available     设置了 GPG_PRIVATE_KEY 就返回 0
#   gpg_setup         把私钥导入一个一次性的 GNUPGHOME（不碰运行环境里已有的密钥环），之后的 gpg 命令都用它。
#                     调用方要在退出时调 gpg_teardown，一般是 trap gpg_teardown EXIT
#   gpg_teardown      让 gpg-agent 退出，并删掉一次性的 GNUPGHOME
#   GPG_SIGN_KEY      gpg_setup 之后：用来签名的密钥（GPG_KEY_ID，没设就是导入的第一把私钥的指纹）
#   GPG_PASSFILE      gpg_setup 之后：口令文件的路径，在 GNUPGHOME 里，随 gpg_teardown 一起删掉

GPG_HOME=""

gpg_available() {
  [ -n "${GPG_PRIVATE_KEY:-}" ]
}

gpg_setup() {
  need gpg
  # macOS 上 gpg-agent 的 socket 路径有长度上限，临时目录要放在短路径下。
  GPG_HOME="$(mktemp -d /tmp/pcs-gnupg.XXXXXX)"
  chmod 700 "$GPG_HOME"
  export GNUPGHOME="$GPG_HOME"

  printf '%s\n' "$GPG_PRIVATE_KEY" | gpg --batch --quiet --import
  GPG_SIGN_KEY="${GPG_KEY_ID:-}"
  if [ -z "$GPG_SIGN_KEY" ]; then
    # --with-colons 的第一条 fpr 记录就是第一把私钥的指纹
    GPG_SIGN_KEY="$(gpg --batch --list-secret-keys --with-colons | awk -F: '$1 == "fpr" { print $10; exit }')"
  fi
  [ -n "$GPG_SIGN_KEY" ] || die "导入 GPG_PRIVATE_KEY 之后没有找到可用的私钥"

  GPG_PASSFILE="${GPG_HOME}/passphrase"
  (
    umask 077
    printf '%s' "${GPG_PASSPHRASE:-}" > "$GPG_PASSFILE"
  )
}

gpg_teardown() {
  [ -n "$GPG_HOME" ] || return 0
  # 先让后台的 gpg-agent / keyboxd 退出，再删目录；它们退出得慢一点时再删一次。
  gpgconf --kill all > /dev/null 2>&1 || true
  rm -rf "$GPG_HOME" 2> /dev/null || {
    sleep 1
    rm -rf "$GPG_HOME"
  }
  GPG_HOME=""
}
