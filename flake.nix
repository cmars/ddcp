{
  description = "ddcp";

  inputs = {
    # NixOS 23.11 has recent enough versions of capnproto and protobuf to
    # develop on Veilid.
    nixpkgs.url = "github:nixos/nixpkgs/nixos-23.11";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    (flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            rust-overlay.overlays.default
          ];
        };

        arch = "x86_64";  # TODO: derive this from system?

      in {

        devShells.default = pkgs.mkShell {
          buildInputs = [
          ] ++ (with pkgs; [
            cargo
            cargo-watch
            rustfmt
            rust-analyzer
            (rust-bin.stable.latest.default.override { extensions = [ "rust-src" ]; })
            clang
            llvmPackages.llvm
            llvmPackages.libclang
            gnumake
            sqlite
            capnproto
            protobuf
          ]);

          LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib";
        };
      }
    ));
}
