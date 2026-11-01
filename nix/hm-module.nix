# home-manager: programs.yamusicrs
self:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.yamusicrs;
  toml = pkgs.formats.toml { };
in
{
  options.programs.yamusicrs = {
    enable = lib.mkEnableOption "yamusicrs — терминальный клиент Яндекс Музыки";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default.override {
        inherit (cfg) desktopTerminal;
      };
      defaultText = lib.literalExpression "yamusicrs.packages.\${system}.default";
      description = "Пакет yamusicrs.";
    };

    desktopTerminal = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "kitty";
      description = ''
        Терминал для ярлыка в меню (запускается как `<терминал> -e yamusicrs`).
        Обложки и блоб рисуются только в kitty/ghostty/WezTerm.
        null — ярлык с Terminal=true, терминал выбирает окружение.
      '';
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = lib.literalExpression ''
        {
          quality = "lossless";
          library_dir = "''${config.home.homeDirectory}/Music/YandexMusic";
          library_max_gb = 50;
          normalize = false;
          fps = 30;
        }
      '';
      description = ''
        Содержимое `~/.config/yamusicrs/config.toml`. Пусто — файл не трогаем,
        плеер сам создаст его с настройками по умолчанию.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];
    xdg.configFile."yamusicrs/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "yamusicrs-config.toml" cfg.settings;
    };
  };
}
