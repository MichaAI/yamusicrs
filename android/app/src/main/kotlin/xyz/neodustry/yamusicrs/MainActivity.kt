package xyz.neodustry.yamusicrs

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import com.google.common.util.concurrent.ListenableFuture

class MainActivity : ComponentActivity() {
    private var controller: ListenableFuture<MediaController>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (Build.VERSION.SDK_INT >= 33) {
            registerForActivityResult(ActivityResultContracts.RequestPermission()) {}
                .launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        askBackground(this, onlyOnce = true)
        val app = application as YaApp
        setContent {
            AppScreen(app, openUrl = { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(it))) })
        }
    }

    override fun onStart() {
        super.onStart()
        // Подключение к сессии поднимает медиасервис: уведомление и фон, когда заиграет.
        controller = MediaController.Builder(this, SessionToken(this, ComponentName(this, PlaybackService::class.java)))
            .buildAsync()
    }

    override fun onStop() {
        controller?.let(MediaController::releaseFuture)
        controller = null
        super.onStop()
    }
}

/** Система не ограничивает приложение в фоне (исключено из оптимизации батареи). */
fun unrestricted(ctx: android.content.Context): Boolean =
    ctx.getSystemService(android.os.PowerManager::class.java).isIgnoringBatteryOptimizations(ctx.packageName)

/**
 * Попросить исключение из оптимизации батареи: без него оболочки вроде vivo/MIUI убивают
 * игру в фоне. `onlyOnce` — при запуске спрашиваем один раз, из настроек — всегда.
 */
@android.annotation.SuppressLint("BatteryLife")
fun askBackground(ctx: android.content.Context, onlyOnce: Boolean) {
    if (unrestricted(ctx)) return
    val prefs = ctx.getSharedPreferences("ui", android.content.Context.MODE_PRIVATE)
    if (onlyOnce && prefs.getBoolean("asked_background", false)) return
    prefs.edit().putBoolean("asked_background", true).apply()
    runCatching {
        ctx.startActivity(
            Intent(android.provider.Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:" + ctx.packageName)),
        )
    }.onFailure {
        runCatching { ctx.startActivity(Intent(android.provider.Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }
    }
}
