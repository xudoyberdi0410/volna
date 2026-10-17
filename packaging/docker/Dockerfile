# Release build environment: Linux bundles on an old glibc (Ubuntu 22.04) and
# the Windows NSIS installer cross-built with cargo-xwin. Used by
# scripts/release-local.sh.
FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl wget file git unzip xz-utils pkg-config \
        libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev \
        libssl-dev patchelf \
        gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav \
        nsis clang lld llvm \
    && rm -rf /var/lib/apt/lists/*

# cargo-xwin calls the unversioned LLVM tools.
RUN for t in clang-cl lld-link llvm-rc llvm-lib llvm-dlltool; do \
        src="$(ls /usr/bin/$t-* 2>/dev/null | head -1 || true)"; \
        [ -e /usr/bin/$t ] || [ -z "$src" ] || ln -s "$src" /usr/bin/$t; \
    done; \
    [ -e /usr/bin/clang-cl ] || ln -s "$(readlink -f /usr/bin/clang)" /usr/bin/clang-cl

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/bun/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup target add x86_64-pc-windows-msvc \
    && cargo install --locked cargo-xwin \
    && chmod -R a+rwX /opt/cargo /opt/rustup

ENV BUN_INSTALL=/opt/bun
RUN curl -fsSL https://bun.sh/install | bash -s "bun-v1.4.3" && chmod -R a+rwX /opt/bun

WORKDIR /src
