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

if [ ! -f "external/cr-sqlite/core/rs/sqlite-rs-embedded/rust-toolchain.toml" ]; then
    # Hydrate an uninitialized submodule
    (cd external/cr-sqlite; git submodule init && git submodule update --recursive)
fi

if [[ "$(pwd)" =~ "/target/package/ddcp-" ]]; then
    # We're almost certain to be in a cargo publish.
    #
    # cargo publish doesn't seem to include the full vendored cr-sqlite project
    # in the package, so we'll copy cr-sqlite from vendored source to a temp
    # directory, build it there, then copy the release artifact to the output.
    build_tmp=$(mktemp -d)
    trap "rm -rf $build_tmp" EXIT
    (cd ../../..; tar cf - external/cr-sqlite) | (cd $build_tmp; tar xf -)
    cd $build_tmp
    make -C external/cr-sqlite/core loadable
    cp $EXT_PATH $_manifest_dir/target/$_profile
    exit 0
fi

if [ ! -f "$EXT_PATH" ]; then
    make -C external/cr-sqlite/core loadable
fi

cp $EXT_PATH $_manifest_dir/target/$_profile
