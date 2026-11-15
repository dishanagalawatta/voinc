{
  description = "VOINC local-first voice dictation";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      package = pkgs: self.packages.${pkgs.stdenv.hostPlatform.system}.voinc;
    in
    {
      packages = forSystems (
        pkgs:
        let
          voinc = pkgs.callPackage ./nix/package.nix { };
        in
        {
          inherit voinc;
          default = voinc;
        }
      );

      overlays.default = final: _prev: {
        voinc = final.callPackage ./nix/package.nix { };
      };

      nixosModules.voinc =
        { pkgs, ... }:
        {
          imports = [ ./nix/module.nix ];
          programs.voinc.package = nixpkgs.lib.mkDefault (package pkgs);
        };
      nixosModules.default = self.nixosModules.voinc;

      homeManagerModules.voinc =
        { pkgs, ... }:
        {
          imports = [ ./nix/hm-module.nix ];
          programs.voinc.package = nixpkgs.lib.mkDefault (package pkgs);
        };
      homeManagerModules.default = self.homeManagerModules.voinc;

      devShells = forSystems (pkgs: {
        default = (package pkgs).devShell;
      });
      checks = forSystems (pkgs: import ./nix/checks.nix { inherit self pkgs; });
      formatter = forSystems (pkgs: pkgs.nixfmt);
    };
}
