{
  description = "ddcp";

  inputs = {
    nixpkgs.url = "nixpkgs";

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
        capnprotoVersion = "1.0.1";
        protobufVersion = "24.3";
        veilidVersion = "0.2.3";

        capnproto_veilid = (with pkgs; stdenv.mkDerivation {
          pname = "capnproto";
          version = "${capnprotoVersion}";
          src = fetchzip {
            url = "https://capnproto.org/capnproto-c++-${capnprotoVersion}.tar.gz";
            hash = "sha256-9y1T1ddGEK8X8qjnh5ZDfqgyPagYghJG+EAwdT83VJs=";
          };
          nativeBuildInputs = [
            autoconf
            automake
            m4
            gnumake
            clang
          ];
          buildPhase = "./configure --without-openssl --prefix=$out && make";
          installPhase = ''
            mkdir -p $out/bin
            make -C $TMP/source install prefix=$out
          '';
        });

        protobuf_veilid = (with pkgs; stdenv.mkDerivation {
          pname = "protobuf";
          version = "${protobufVersion}";
          src = fetchzip {
            stripRoot = false;
            url = "https://github.com/protocolbuffers/protobuf/releases/download/v${protobufVersion}/protoc-${protobufVersion}-linux-${arch}.zip";
            hash = "sha256-QrJi/s6DEPbX/1k64UFJxzq7SUwapDjuWU5UNrAFMlI=";
          };
          buildPhase = ''
            chmod +x bin/*
          '';
          installPhase = ''
            mkdir -p $out/bin $out/include
            cp -r $TMP/source/bin/* $out/bin
            cp -r $TMP/source/include/* $out/include
          '';
        });

        veilid_src = (with pkgs; stdenv.mkDerivation {
          pname = "veilid";
          version = "${veilidVersion}";
          src = fetchgit {
            url = "https://gitlab.com/veilid/veilid.git";
            rev = "v${veilidVersion}";
            hash = "sha256-fpA0JsBp2mlyDWlwE6xgyX5KNI2FSypO6m1J9BI+Kjs=";
            fetchSubmodules = true;
          };
          buildPhase = ''
            ls
          '';
          installPhase = ''
            mkdir -p $out
            tar cf - . | (cd $out; tar xf -)
          '';
        });

      in {

        devShells.default = pkgs.mkShell {
          buildInputs = [
            capnproto_veilid
            protobuf_veilid
            veilid_src
          ] ++ (with pkgs; [
            cargo
            cargo-watch
            rustfmt
            rust-analyzer
            rust-bin.nightly.latest.default
            clang
            llvmPackages.llvm
            llvmPackages.libclang
            gnumake
            sqlite
          ]);

          VEILID_SRC="${veilid_src}";
          LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib";

          shellHook = ''
          '';
        };
      }
    ));
}
