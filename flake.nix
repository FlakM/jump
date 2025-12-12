{
  description = "Jump - code navigation tool for tmux/neovim workflows";

  inputs = {
    nixpkgs.url = "nixpkgs/nixos-25.11";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, flake-utils, rust-overlay, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default;
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;
          buildInputs = with pkgs; [
            zlib
            libgit2
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];
          nativeBuildInputs = with pkgs; [
            pkg-config
            git    # needed for git integration tests
            tmux   # needed for tmux integration tests
          ];
        };

        # neovim only needed for build-time tests, not in devshell
        # (devshell should use user's configured neovim from home-manager)
        buildArgs = commonArgs // {
          nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.neovim ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        jumpBin = craneLib.buildPackage (buildArgs // {
          inherit cargoArtifacts;
        });

        jump = pkgs.runCommand "jump-with-completions" {
          nativeBuildInputs = [ pkgs.installShellFiles ];
        } ''
          mkdir -p $out/bin
          cp ${jumpBin}/bin/jump $out/bin/

          installShellCompletion --cmd jump \
            --bash <($out/bin/jump completions bash) \
            --zsh <($out/bin/jump completions zsh) \
            --fish <($out/bin/jump completions fish)
        '';

      in
      {
        packages = {
          default = jump;
          jump = jump;
        };

        # devShell without neovim - use user's home-manager nvim instead
        devShells.default = craneLib.devShell {
          checks = { inherit cargoArtifacts; };
          packages = with pkgs; [
            rust-analyzer
          ];
          env = {
            LIBGIT2_SYS_USE_PKG_CONFIG = "1";
          };
        };
      }
    );
}
