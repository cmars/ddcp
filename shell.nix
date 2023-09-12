{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  buildInputs = with pkgs.buildPackages; [
    rustup
    clang
    llvmPackages.llvm
    llvmPackages.libclang
    gnumake
    sqlite
  ];

  LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib";

  shellHook = ''
  '';
}

