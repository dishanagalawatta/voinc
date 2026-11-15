{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.voinc;
in
{
  options.programs.voinc = {
    enable = lib.mkEnableOption "Voinc local voice dictation";
    package = lib.mkOption {
      type = lib.types.package;
      description = "Voinc package to install.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.elem pkgs.stdenv.hostPlatform.system [
          "x86_64-linux"
          "aarch64-linux"
        ];
        message = "The Voinc Nix package supports x86_64-linux and aarch64-linux only.";
      }
    ];
    environment.systemPackages = [ cfg.package ];
  };
}
