package xyz.neodustry.yamusicrs

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import uniffi.yamusic_android.Core
import kotlin.math.abs
import kotlin.math.log10
import kotlin.math.pow
import kotlin.math.roundToInt

const val EQ_RANGE = 12f
val EQ_BANDS = listOf(5, 10, 15, 31)

/** Центры полос так же, как в ядре (`dsp::eq_freqs`). */
fun eqFreqs(n: Int): List<Float> =
    if (n == 10) listOf(31f, 62f, 125f, 250f, 500f, 1000f, 2000f, 4000f, 8000f, 16000f)
    else List(n) { i -> 31f * (16000f / 31f).pow(if (n == 1) 0.5f else i / (n - 1f)) }

fun freqLabel(f: Float): String = if (f >= 1000) "${(f / 1000).let { if (it == it.toInt().toFloat()) it.toInt().toString() else "%.1f".format(it) }}к" else f.roundToInt().toString()

/** Ряд вертикальных ползунков эквалайзера: по одному на полосу, двойное касание сбрасывает полосу. */
@Composable
fun EqBands(core: Core, gains: List<Float>, enabled: Boolean) {
    val n = gains.size
    val freqs = remember(n) { eqFreqs(n) }
    val scroll = rememberScrollState()
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val colW: Dp = maxOf(maxWidth / n, 30.dp)
        Row(Modifier.horizontalScroll(scroll)) {
            for (i in 0 until n) {
                EqBand(colW, freqs[i], gains[i], enabled, showLabel = n <= 15 || i % 3 == 0) { core.setEqGain(i.toUInt(), it) }
            }
        }
    }
}

@Composable
private fun EqBand(width: Dp, freq: Float, gain: Float, enabled: Boolean, showLabel: Boolean, set: (Float) -> Unit) {
    var local by remember(gain) { mutableFloatStateOf(gain) }
    val track = MaterialTheme.colorScheme.surfaceContainerHighest
    val accent = if (enabled) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline
    Column(Modifier.width(width), horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            if (abs(local) < 0.05f) "0" else "%+.0f".format(local),
            style = MaterialTheme.typography.labelSmall,
            color = if (abs(local) < 0.05f) MaterialTheme.colorScheme.onSurfaceVariant else accent,
            maxLines = 1,
        )
        Canvas(
            Modifier.height(180.dp).width(width).pointerInput(Unit) {
                // Касание ставит значение под пальцем, перетаскивание ведёт его; уровень уходит в ядро на отпускании.
                fun at(y: Float) = ((0.5f - (y / size.height)) * 2 * EQ_RANGE).coerceIn(-EQ_RANGE, EQ_RANGE).roundToInt().toFloat()
                detectVerticalDragGestures(
                    onDragStart = { local = at(it.y) },
                    onDragEnd = { set(local) },
                    onDragCancel = { set(local) },
                ) { change, _ -> local = at(change.position.y) }
            }.pointerInput(Unit) {
                detectTapGestures(onDoubleTap = { local = 0f; set(0f) }, onTap = { local = ((0.5f - it.y / size.height) * 2 * EQ_RANGE).coerceIn(-EQ_RANGE, EQ_RANGE).roundToInt().toFloat(); set(local) })
            },
        ) {
            val tw = 8.dp.toPx()
            val x = (size.width - tw) / 2
            drawRoundRect(track, Offset(x, 0f), Size(tw, size.height), CornerRadius(tw / 2))
            val mid = size.height / 2
            val y = mid - local / EQ_RANGE * mid
            drawRoundRect(accent, Offset(x, minOf(y, mid)), Size(tw, abs(y - mid).coerceAtLeast(1f)), CornerRadius(tw / 2))
            drawCircle(accent, 11.dp.toPx(), Offset(size.width / 2, y))
        }
        Text(
            if (showLabel) freqLabel(freq) else "",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            maxLines = 1,
            softWrap = false,
        )
    }
}

@Composable
fun EqReset(core: Core, gains: List<Float>) {
    OutlinedButton(onClick = { core.resetEq() }, enabled = gains.any { abs(it) > 0.05f }) { Text("Сбросить") }
}
