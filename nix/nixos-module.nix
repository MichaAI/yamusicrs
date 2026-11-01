# NixOS: programs.yamusicrs
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.yamusicrs;
in
{
  options.programs.yamusicrs = {
    enable = lib.mkEnableOption "yamusicrs — терминальный клиент Яндекс Музыки";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "yamusicrs.packages.\${system}.default";
      description = "Пакет yamusicrs.";
    };

    hiResRates = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Разрешить PipeWire работать на частоте исходника (44.1–192 кГц),
        чтобы lossless не ресемплился в 48 кГц.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    services.pipewire.extraConfig.pipewire."92-yamusicrs-hires" = lib.mkIf cfg.hiResRates {
      "context.properties"."default.clock.allowed-rates" = [
        44100
        48000
        88200
        96000
        176400
        192000
      ];
    };
  };
}
