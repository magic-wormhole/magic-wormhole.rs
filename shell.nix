# visit https://status.nixos.org/ and pick the latest commit for
# nixpkgs-unstable to update the nixpkgs hash
{ pkgs ? import (fetchTarball "https://github.com/NixOS/nixpkgs/archive/1882c6b7368fd284ad01b0a5b5601ef136321292.tar.gz") {}
}:
pkgs.mkShell {
  nativeBuildInputs = with pkgs; [
    bashInteractive
    rustc
    cargo
    rustfmt
  ];
}
