### Build

FROM docker.io/library/debian:bookworm AS builder

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq && \
    apt-get install -y \
        build-essential \
        ca-certificates \
        curl \
        git \
        lsb-release \
        patchutils \
        unzip \
        --no-install-recommends

WORKDIR /src/vendor
RUN git clone https://gitlab.com/veilid/veilid.git --depth 1 -b v0.2.3 veilid

WORKDIR /src/vendor/veilid

# Install rustup, use nightly. crsqlite needs nightly.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly

# Install serde tooling
RUN bash -xe scripts/earthly/install_capnproto.sh
RUN bash -xe scripts/earthly/install_protoc.sh

# Build ddcp
WORKDIR /src
COPY . .
ENV PATH="/root/.cargo/bin:${PATH}"
RUN cargo build --release

### Runtime

FROM gcr.io/distroless/static-debian12
COPY --from=builder /src/target/release/ddcp /ddcp
COPY --from=builder /src/target/release/crsqlite.so /crsqlite.so
ENV DB_FILE /data/db
ENV STATE_DIR /data/state
ENV EXT_FILE /crsqlite.so
VOLUME /data
ENTRYPOINT ["/ddcp"]
