plugins {
    id("com.android.application") version "9.5.0" apply false
    // Kotlin встроен в AGP 9; плагин Compose задаёт и версию Kotlin.
    id("org.jetbrains.kotlin.plugin.compose") version "2.4.10" apply false
}
