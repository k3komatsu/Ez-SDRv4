# The image the remote mutation shards run in (remote_mutate.py). Built on the development image
# (docker/uhd4.10/Dockerfile: Rust stable, libuhd 4.10, Python 3 with numpy), so a row can build
# `ezsdr-radio-uhd` with `--features uhd`; each host needs `ezsdr-v4-dev:uhd4.10` built (a missing one
# makes this build try to pull it). Stable is the dev image's default toolchain, which mutate.py's
# `cargo +stable` names. cargo-mutants is for the generator sweep (sweep_mutants.py) only. Not root: a
# test that makes a directory read-only to fail a write (G20's) passes as root, and the dev image runs
# as root by default.
FROM ezsdr-v4-dev:uhd4.10
RUN apt-get update && apt-get install -y --no-install-recommends rsync \
 && rm -rf /var/lib/apt/lists/* \
 && cargo install cargo-mutants --locked --version 27.1.0 --root /usr/local \
 && rm -rf /usr/local/cargo/registry \
 && useradd -m mut
USER mut
WORKDIR /src
