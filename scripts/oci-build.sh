#!/usr/bin/env bash

set -eux

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && \
    apt-get install -y ca-certificates curl git lsb-release patchutils --no-install-recommends

pushd vendor
rm -rf veilid
git clone https://gitlab.com/veilid/veilid.git --depth 1 -b v0.2.3 veilid
popd  # vendor

pushd vendor/veilid
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly

# Install serde tooling
bash -xe scripts/earthly/install_capnproto.sh
bash -xe scripts/earthly/install_protoc.sh
popd  # vendor/veilid

cargo build --release
