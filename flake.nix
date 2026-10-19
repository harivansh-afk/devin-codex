{
  description = "Rust adapter for Devin inference";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/c5c4a43b0e8056328ec4529f735cabdb8f1942bb";
  inputs.codex-nixpkgs.url = "github:NixOS/nixpkgs/02f5696b0e6097e589076d886b317b83ff0437d7";

  outputs =
    {
      self,
      nixpkgs,
      codex-nixpkgs,
    }:
    let
      systems = [
        "aarch64-linux"
        "x86_64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];
      eachSystem = nixpkgs.lib.genAttrs systems;
      package =
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        pkgs.rustPlatform.buildRustPackage {
          pname = "devin-codex";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./src
              ./tests
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          preCheck = ''
            export SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
          '';
          meta.mainProgram = "devin-codex";
        };
    in
    {
      packages = eachSystem (
        system:
        {
          default = package system;
          devin-codex = self.packages.${system}.default;
        }
        // nixpkgs.lib.optionalAttrs (system != "x86_64-darwin") {
          codex = codex-nixpkgs.legacyPackages.${system}.callPackage ./nix/codex.nix { };
          with-terminal-theme = nixpkgs.legacyPackages.${system}.symlinkJoin {
            name = "devin-codex-with-terminal-theme";
            paths = [ self.packages.${system}.default ];
            nativeBuildInputs = [ nixpkgs.legacyPackages.${system}.makeWrapper ];
            postBuild = ''
              wrapProgram $out/bin/devin-codex \
                --prefix PATH : ${self.packages.${system}.codex}/bin
            '';
            meta.mainProgram = "devin-codex";
          };
        }
      );
      apps = eachSystem (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/devin-codex";
        };
      });
      checks = eachSystem (
        system:
        {
          package = self.packages.${system}.default;
        }
        // nixpkgs.lib.optionalAttrs (system != "x86_64-darwin") {
          terminal-theme =
            nixpkgs.legacyPackages.${system}.runCommand "terminal-theme"
              {
                nativeBuildInputs = [
                  (nixpkgs.legacyPackages.${system}.python3.withPackages (ps: [ ps.pyte ]))
                ];
              }
              ''
                python3 ${./tests/terminal_theme.py} ${self.packages.${system}.codex}/bin/codex
                touch $out
              '';
        }
      );
      devShells = eachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
            ];
          };
        }
      );
      formatter = eachSystem (system: nixpkgs.legacyPackages.${system}.nixfmt);
    };
}
