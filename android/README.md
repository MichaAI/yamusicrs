# Android-приложение

Kotlin + Jetpack Compose (Material 3 Expressive). Вся логика — в Rust-ядре
(`crates/core`), которое подключается через UniFFI; звук играет AAudio, медиа-уведомление
и сессия — Media3.

- `applicationId`: `xyz.neodustry.yamusicrs`, `minSdk` 28, `targetSdk` 36, `compileSdk` 37.2
- ABI по умолчанию: `arm64-v8a` и `x86_64` (последний — для эмулятора/Waydroid)

## Сборка через Nix (проще всего)

```sh
nix develop .#android -c ./android/gradlew -p android assembleRelease
```

Оболочка `android` даёт JDK 21, Android SDK (build-tools 36.1.0, платформа 37.2),
NDK 29.0.14206865, `cargo-ndk`, `cmake`, Rust с таргетами Android и `adb`.
Результат: `android/app/build/outputs/apk/release/app-release.apk`.

Только нужная архитектура (быстрее):

```sh
nix develop .#android -c ./android/gradlew -p android assembleRelease -Pabis=arm64-v8a
```

Установка:

```sh
adb install -r android/app/build/outputs/apk/release/app-release.apk
```

Сборка **release** — основная: в debug-варианте Compose без R8 заметно тормозит.
Release подписан debug-ключом, поэтому ставится поверх прежней сборки без потери входа.

### Отладка нативной части

`-Pdebuggable` собирает тот же release, но debuggable и с символами в `.so`
(можно цепляться lldb через `run-as` + `lldb-server`). ART в таком режиме медленнее —
только для ловли багов.

```sh
nix develop .#android -c ./android/gradlew -p android assembleRelease -Pdebuggable
adb logcat -s yamusicrs      # лог ядра идёт в logcat (crates/android/src/logcat.rs)
```

## Сборка без Nix

Нужны:

1. JDK 21, Android SDK с платформой 37.2 и build-tools 36.1.0 (версии в `app/build.gradle.kts`),
   NDK 29.0.14206865; переменные `ANDROID_HOME`, `ANDROID_NDK_HOME`, `JAVA_HOME`.
2. Rust (stable) и таргеты: `rustup target add aarch64-linux-android x86_64-linux-android`.
3. `cargo install cargo-ndk`, `cmake`.

```sh
cd android && ./gradlew assembleRelease
```

Gradle-задача `cargoNdk` перед компиляцией Kotlin сама:

1. запускает `cargo ndk … build --release -p yamusic-android` (результат — `libyamusic_android.so`
   под каждый ABI, кладётся в jniLibs; кеш cargo в `target/android`);
2. запускает `uniffi-bindgen generate --library …` — Kotlin-привязки генерируются
   из собранной библиотеки и подключаются как сгенерированные исходники.

Поэтому правка Rust-кода достаточна для пересборки — отдельных шагов не нужно.
Привязки руками в репозиторий не кладутся.

## Структура

```
app/src/main/kotlin/xyz/neodustry/yamusicrs/
  MainActivity.kt   вход, навигация, жесты
  YaApp.kt          корневой Compose-экран, вкладки, мини-плеер
  Pages.kt          страницы: плейлист, альбом, артист, поиск…
  Ui.kt, Expressive.kt, Motion.kt, Blob.kt, Palette.kt, Icons.kt, Avatars.kt
                    компоненты, анимации, динамическая палитра из обложки
  CorePlayer.kt     обёртка над uniffi-объектом Core, слушатель состояния
  PlaybackService.kt  Media3-сессия, уведомление, медиакнопки
app/src/main/res/   иконки, тема, шрифт Nunito (лицензия — assets/licenses)
```

## Вход и данные

Вход — по коду (как в TUI). Токен и библиотека лежат во внутреннем хранилище приложения.
Состояние, очередь и «Ynison» работают так же, как на десктопе: телефон виден
в «Слушать на…» и может перехватывать воспроизведение.

## Частые проблемы

- **`aapt2` не запускается на NixOS** — оболочка `.#android` уже подставляет aapt2 из SDK через
  `GRADLE_OPTS`; вне её задайте `-Dorg.gradle.project.android.aapt2FromMavenOverride=…/aapt2`.
- **Gradle пытается докачать SDK** — store только для чтения; используйте ровно те версии,
  что в `flake.nix` и `build.gradle.kts`.
- **Нехватка памяти при сборке** — `org.gradle.jvmargs` в `gradle.properties` (по умолчанию 3 ГБ).
