# Linux 版的构建环境，由 scripts/linux/docker.sh 构建并使用（构建上下文是 scripts/ 目录）。
# 基础镜像用 Ubuntu 22.04：编出的二进制要求 glibc >= 2.35，和 CI 的 ubuntu-22.04 runner 一致。
FROM ubuntu:22.04

# 系统依赖和打包工具。只复制装依赖要用的两个脚本，别的脚本改了不会让这一层的缓存失效。
COPY lib/common.sh /opt/pc-specs/scripts/lib/common.sh
COPY linux/install-deps.sh /opt/pc-specs/scripts/linux/install-deps.sh
RUN /opt/pc-specs/scripts/linux/install-deps.sh --package \
    && rm -rf /var/lib/apt/lists/*

# Rust 工具链放在镜像里；cargo 的下载缓存和 target/ 在运行时挂 volume（见 docker.sh）。
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo \
    PATH=/opt/rust/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
