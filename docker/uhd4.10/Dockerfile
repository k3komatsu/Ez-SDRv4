# Ez-SDR v4 development environment on UHD 4.10 (Ubuntu 26.04, linux/amd64).
#
# The base is the UHD image v3's docker/v3_prebuild builds on: UHD 4.10.0.0 built from
# source into /usr/local, its FPGA images downloaded, Python 3 with numpy. This adds Rust
# (stable and the MSRV) and nothing else; the source is mounted, not copied, and the build
# output goes to /cargo-target, outside the tree (AGENTS.md §7).
#
#   docker build -t ezsdr-v4-dev:uhd4.10 docker/uhd4.10
#   docker run -it --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 \
#       --user "$(id -u):$(id -g)" -v "$PWD:/work" -v ezsdr-v4-cargo-target:/cargo-target \
#       ezsdr-v4-dev:uhd4.10
#
# --network=host reaches the X310 over its 10 GbE link; SYS_NICE and rtprio let UHD raise its
# streaming threads' priority. The host still sets the NIC's MTU and net.core.{r,w}mem_max
# (bench.md B0.2): those are not the container's. .devcontainer/devcontainer.json uses this file.
ARG UHD_VERSION=v4.10
FROM ghcr.io/k3komatsu/uhd:${UHD_VERSION}

# Cargo.toml's rust-version.
ARG RUST_MSRV=1.85.0

# The base image puts /usr/local/lib on LD_LIBRARY_PATH only in /root/.bashrc, and build.rs
# adds no rpath (UR-35): register libuhd for every process and user.
RUN ldconfig

ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN <<EOF
    set -e
    curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain stable --component clippy,rustfmt
    rustup toolchain install ${RUST_MSRV} --profile minimal --component clippy
    # Any user (the devcontainer's, or docker run --user) fetches crates and builds.
    chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"
    mkdir -p /cargo-target && chmod 1777 /cargo-target
EOF

ENV CARGO_TARGET_DIR=/cargo-target \
    EZSDR_SERVER=/cargo-target/debug/ezsdr-server \
    PYTHONDONTWRITEBYTECODE=1
WORKDIR /work
CMD ["/bin/bash"]
