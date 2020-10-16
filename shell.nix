let
  moz_overlay = import (
    builtins.fetchGit {
      url = "git@github.com:mozilla/nixpkgs-mozilla.git";
      rev = "efda5b357451dbb0431f983cca679ae3cd9b9829";
    }
  );
  
  pkgs = import <nixpkgs> {
    overlays = [ moz_overlay ];
  };

  # rust = pkgs.rustChannels.nightly.rust.override {
  #   extensions = [ "rust-src" ];
  # };
  rust = pkgs.rustChannels.nightly.rust.override {
    extensions = [ "rust-src" "rls-preview"
                   "rust-analysis" "rustfmt-preview" ];
  };
  
  rust-path = pkgs.stdenv.mkDerivation {
    inherit (pkgs.rustc) src;
    inherit (pkgs.rustc.src) name;
    phases = ["unpackPhase" "installPhase"];
    installPhase = "cp -r src $out";
  };

  # rust-channel = pkgs

  # cargo = rust.cargo;
in
pkgs.mkShell {
  name = "genkfs-rust";
  nativeBuildInputs = [ rust pkgs.rustracer ];
  RUST_SRC_PATH = rust-path;
  src = ./.;
}
