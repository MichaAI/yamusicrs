package xyz.neodustry.yamusicrs

import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import uniffi.yamusic_android.Core
import kotlin.math.min
import kotlin.math.pow

/** Что рисовать на фоне и откуда брать спектр; обновляется из настроек. */
object Viz {
    var spectrum by androidx.compose.runtime.mutableStateOf(false)
    var core: Core? = null
}

private const val BARS = 32

/**
 * Спектр вместо блоба: столбики растут от нижнего края, цвета — из палитры обложки.
 * Уровни читаются прямо из потока вывода; кадр перерисовывается только в фазе рисования.
 */
@Composable
fun SpectrumBackdrop(colors: List<Color>, playing: Boolean, modifier: Modifier = Modifier) {
    val levels = remember { FloatArray(BARS) }
    val target = remember { FloatArray(BARS) }
    var tick by remember { mutableIntStateOf(0) }
    val play by rememberUpdatedState(playing)
    LaunchedEffect(Unit) {
        var last = 0L
        while (true) {
            withFrameNanos { now ->
                val dt = if (last == 0L) 0f else ((now - last) / 1e9f).coerceAtMost(0.1f)
                last = now
                val raw = if (play) Viz.core?.spectrum().orEmpty() else emptyList()
                for (i in 0 until BARS) {
                    // 64 полос ядра → BARS столбцов (максимум из пары).
                    target[i] = if (raw.size >= BARS * 2) maxOf(raw[i * 2], raw[i * 2 + 1]).pow(1.3f) else 0f
                }
                // Сглаживание по соседям, затем плавный подъём и более медленный спад по времени кадра.
                val up = min(dt * 14f, 1f)
                val down = min(dt * 4f, 1f)
                for (i in 0 until BARS) {
                    val t = 0.25f * target[maxOf(i - 1, 0)] + 0.5f * target[i] + 0.25f * target[min(i + 1, BARS - 1)]
                    levels[i] += (t - levels[i]) * (if (t > levels[i]) up else down)
                }
                tick++
            }
        }
    }
    val base = colors.firstOrNull() ?: Color(0xFF202020)
    val top = colors.getOrNull(1) ?: base
    val bottom = colors.getOrNull(2) ?: top
    Canvas(modifier) {
        tick.let { }
        drawRect(Brush.verticalGradient(listOf(base.copy(alpha = 0.35f), base.copy(alpha = 0.12f))))
        val gap = 6.dp.toPx()
        val w = (size.width - gap * (BARS + 1)) / BARS
        val maxH = size.height * 0.62f
        for (i in 0 until BARS) {
            val h = (levels[i] * maxH).coerceAtLeast(w)
            val x = gap + i * (w + gap)
            drawRoundRect(
                Brush.verticalGradient(listOf(top, bottom), startY = size.height - h, endY = size.height),
                topLeft = Offset(x, size.height - h),
                size = Size(w, h),
                cornerRadius = CornerRadius(w / 2, w / 2),
                alpha = 0.55f,
            )
        }
    }
}
