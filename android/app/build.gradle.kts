import javax.inject.Inject

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "xyz.neodustry.yamusicrs"
    // Свежие AndroidX (Compose 1.12, lifecycle 2.11) требуют компиляции против API 37.
    compileSdk {
        version = release(37) { minorApiLevel = 2 }
    }
    // Та же версия, что в nix-SDK (flake.nix): store только для чтения, AGP не докачает свою.
    buildToolsVersion = "36.1.0"

    defaultConfig {
        applicationId = "xyz.neodustry.yamusicrs"
        // AAudio с usage/content type — с Android 9.
        minSdk = 28
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        // Compose в debuggable-сборке без R8 в разы медленнее (списки дёргаются) — играть
        // стоит с release. Подпись — та же debug: ставится поверх без потери входа.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName("debug")
            // `-Pdebuggable` — тот же release, но к нему цепляется lldb (run-as + lldb-server),
            // а в .so остаются символы. Только для ловли багов: ART в debuggable-режиме медленнее.
            isDebuggable = project.hasProperty("debuggable")
        }
    }

    packaging {
        jniLibs {
            if (project.hasProperty("debuggable")) keepDebugSymbols += "**/*.so"
        }
    }

    buildFeatures {
        compose = true
    }

    // lintVital на каждой release-сборке съедал ~50 с из ~110; сборка тут для себя, lint при желании: ./gradlew lint.
    lint {
        checkReleaseBuilds = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

/**
 * Ядро на Rust: `cargo ndk` собирает libyamusic_android.so под нужные ABI, uniffi-bindgen
 * генерирует по ней Kotlin-привязки. Cargo сам решает, что пересобирать, поэтому задача
 * запускается всегда. ABI: `-Pabis=arm64-v8a` (по умолчанию ещё x86_64 — для Waydroid).
 */
abstract class CargoNdk @Inject constructor(private val exec: ExecOperations) : DefaultTask() {
    @get:Input abstract val abis: ListProperty<String>
    @get:Input abstract val repo: Property<String>
    @get:OutputDirectory abstract val jniOut: DirectoryProperty
    @get:OutputDirectory abstract val kotlinOut: DirectoryProperty

    @TaskAction
    fun build() {
        val root = File(repo.get())
        val target = File(System.getenv("CARGO_TARGET_DIR")?.let { if (File(it).isAbsolute) it else "${root}/$it" }
            ?: "${root}/target/android")
        val triples = mapOf("arm64-v8a" to "aarch64-linux-android", "x86_64" to "x86_64-linux-android")
        exec.exec {
            workingDir = root
            environment("CARGO_TARGET_DIR", target.absolutePath)
            commandLine(
                listOf("cargo", "ndk", "--platform", "28", "-o", jniOut.get().asFile.absolutePath) +
                    abis.get().flatMap { listOf("-t", it) } +
                    listOf("build", "--release", "-p", "yamusic-android"),
            )
        }
        val so = File(target, "${triples.getValue(abis.get().first())}/release/libyamusic_android.so")
        exec.exec {
            workingDir = root
            environment("CARGO_TARGET_DIR", target.absolutePath)
            commandLine(
                "cargo", "run", "--release", "-q", "-p", "uniffi-bindgen", "--",
                "generate", "--library", so.absolutePath, "--language", "kotlin",
                "--out-dir", kotlinOut.get().asFile.absolutePath, "--no-format",
            )
        }
    }
}

val cargoNdk = tasks.register<CargoNdk>("cargoNdk") {
    abis.set((findProperty("abis") as String? ?: "arm64-v8a,x86_64").split(",").map(String::trim))
    repo.set(rootProject.projectDir.parentFile.absolutePath)
    outputs.upToDateWhen { false }
}

androidComponents {
    onVariants { v ->
        v.sources.jniLibs?.addGeneratedSourceDirectory(cargoNdk, CargoNdk::jniOut)
        v.sources.java?.addGeneratedSourceDirectory(cargoNdk, CargoNdk::kotlinOut)
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2026.09.00")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.foundation:foundation")
    // Material 3 Expressive (формы, морфинг, волнистый прогресс) — пока только в 1.5.
    implementation("androidx.compose.material3:material3:1.5.0-beta02")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    // Ставит baseline-профиль при первом запуске: без него adb install (не Play) профиль не применяет.
    implementation("androidx.profileinstaller:profileinstaller:1.4.1")
    implementation("androidx.media3:media3-session:1.11.1")
    implementation("androidx.media3:media3-common:1.11.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    // UniFFI-привязки ходят в .so через JNA.
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
