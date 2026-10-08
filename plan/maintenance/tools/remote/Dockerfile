# The image the remote mutation shards run in (remote_mutate.py). Rust stable for
# mutate.py's `cargo +stable`, and Python with numpy for the Phase 6 rows.
FROM rust:1-bookworm
RUN rustup toolchain install stable --profile minimal \
 && apt-get update && apt-get install -y --no-install-recommends python3 python3-numpy rsync \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
