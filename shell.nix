{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
    nativeBuildInputs = [
      pkgs.cargo
      pkgs.rustfmt
      pkgs.rustc
    ];
}
