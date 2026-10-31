{
  description = "Strata, a keyboard-first GTK4 file manager for Linux";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          strata = pkgs.callPackage ./package.nix {
            buildCommit = self.shortRev or self.dirtyShortRev or null;
          };
        in
        {
          inherit strata;
          default = strata;
        }
      );

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) strata;
      });
    };
}
