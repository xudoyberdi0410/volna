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
        nsis clang lld llvm msitools \
    && rm -rf /var/lib/apt/lists/*

# The MSVC STL that cargo-xwin fetches wants Clang 19+; 22.04 has 14.
RUN mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key -o /etc/apt/keyrings/llvm.asc \
    && echo "deb [signed-by=/etc/apt/keyrings/llvm.asc] http://apt.llvm.org/jammy/ llvm-toolchain-jammy-19 main" \
        >/etc/apt/sources.list.d/llvm.list \
    && apt-get update && apt-get install -y --no-install-recommends clang-19 lld-19 llvm-19 \
    && rm -rf /var/lib/apt/lists/*

# cargo-xwin calls the unversioned LLVM tools.
RUN for t in clang clang-cl lld-link llvm-rc llvm-lib llvm-dlltool llvm-objdump; do \
        ln -sf /usr/lib/llvm-19/bin/$t /usr/bin/$t; \
    done

ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/opt/bun/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup target add x86_64-pc-windows-msvc \
    && cargo install --locked cargo-xwin \
    && chmod -R a+rwX /opt/cargo /opt/rustup

ENV BUN_INSTALL=/opt/bun
RUN curl -fsSL https://bun.sh/install | bash -s "bun-v1.4.3" && chmod -R a+rwX /opt/bun

WORKDIR /src

# Tauri's NSIS template includes Win\RestartManager.nsh, which NSIS 3.08 from
# 22.04 does not ship yet.
RUN curl -fsSL https://raw.githubusercontent.com/kichik/nsis/v311/Include/Win/RestartManager.nsh \
        -o /usr/share/nsis/Include/Win/RestartManager.nsh

# 7z unpacks the VC++ runtime out of vc_redist.x64.exe (scripts/release-local.sh).
RUN apt-get update && apt-get install -y --no-install-recommends p7zip-full python3 \
    && rm -rf /var/lib/apt/lists/*
