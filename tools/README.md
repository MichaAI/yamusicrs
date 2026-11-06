# tools

Вспомогательные скрипты для разработки; в пакет не входят.

## ynison_probe.py

Регистрируется на Ynison-хабе пассивным устройством (пустая очередь, `is_currently_active: false`),
не перехватывает воспроизведение и печатает/сохраняет сообщения хаба.

```sh
nix shell nixpkgs#python3Packages.aiohttp -c \
  python3 tools/ynison_probe.py ~/.local/share/yamusicrs/token.json 15 out.json [DEVICE_ID]
```

Аргументы: путь к `token.json` (с `access_token`), секунды прослушивания, необязательно —
файл для дампа и id устройства. Rust-версия того же — `crates/core/examples/ynison_probe.rs`.
Протокол описан в [docs/ynison.md](../docs/ynison.md).
