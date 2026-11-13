package xyz.neodustry.yamusicrs

import android.graphics.Bitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.SemanticsNodeInteraction
import java.io.File

/** Снимок узла в PNG: `app/build/shots/<name>.png`. */
fun SemanticsNodeInteraction.shot(name: String) {
    val dir = File(System.getProperty("shots.dir") ?: "build/shots").apply { mkdirs() }
    File(dir, "$name.png").outputStream().use {
        captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it)
    }
}
