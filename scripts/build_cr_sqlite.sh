#!/usr/bin/env bash
set -eux

_manifest_dir=$CARGO_MANIFEST_DIR
_profile=$PROFILE

unset RUSTC
unset RUSTUP_HOME
unset RUSTUP_TOOLCHAIN
unset CARGO
unset CARGO_PKG_NAME
unset CARGO_PKG_VERSION
unset CARGO_MANIFEST_DIR

EXT_PATH=external/cr-sqlite/core/dist/crsqlite.so

if [ ! -f "$EXT_PATH" ]; then
    make -C external/cr-sqlite/core loadable
fi

cp $EXT_PATH $_manifest_dir/target/$_profile
