### Build

FROM docker.io/library/debian:bookworm AS builder

# Install Debian build dependencies
RUN apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y \
        build-essential \
        ca-certificates \
        curl \
        git \
        lsb-release \
        patchutils \
        unzip \
        libclang-dev \
        --no-install-recommends

# Install rustup, use nightly. crsqlite needs nightly.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /src
COPY external/ external/

# Install serde tooling, needed by veilid-core dependency during build
WORKDIR /src/external/veilid
RUN bash -xe scripts/earthly/install_capnproto.sh
RUN bash -xe scripts/earthly/install_protoc.sh

# Cache ddcp crate dependencies
WORKDIR /src
RUN mkdir -p src/bin
RUN echo 'fn main() {panic!("placeholder")}' > src/lib.rs
RUN echo 'fn main() {panic!("placeholder")}' > src/bin/main.rs
RUN echo 'fn main() {}' > build.rs
COPY ["Cargo.toml", "Cargo.lock", "./"]
RUN cargo build --release

# Build ddcp
COPY . .
RUN touch src/lib.rs src/bin/main.rs build.rs
RUN cargo build --release

### Runtime

FROM debian:bookworm-slim
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y sqlite3
COPY --from=builder /src/target/release/ddcp /usr/bin/ddcp
COPY --from=builder /src/target/release/crsqlite.so /usr/lib/crsqlite.so
ENV DB_FILE /data/db
ENV STATE_DIR /data/state
ENV EXT_FILE /usr/lib/crsqlite.so
VOLUME /data
ENTRYPOINT ["/usr/bin/ddcp"]
