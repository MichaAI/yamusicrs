@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package xyz.neodustry.yamusicrs

import android.graphics.Bitmap
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.toPath
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Matrix
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.graphics.shapes.Morph
import kotlin.random.Random
import uniffi.yamusic_android.PlayerState

/** Глифы перебора — те же, что в TUI. */
private const val GLYPHS = "░▒/\\<>=+*#%_~:·¦"

/**
 * Эффект TUI «перебор символов»: при смене текста буквы проявляются слева направо
 * (с небольшим разбросом на символ) за 650 мс, до этого на их месте мелькают глифы.
 */
@Composable
fun ScrambleText(
    text: String,
    style: TextStyle,
    color: Color,
    modifier: Modifier = Modifier,
    maxLines: Int = 1,
) {
    var shown by remember { mutableStateOf(text) }
    var scrambling by remember { mutableStateOf(false) }
    LaunchedEffect(text) {
        val jitter = FloatArray(text.length) { Random.nextFloat() }
        val start = withFrameNanos { it }
        scrambling = true
        while (true) {
            val now = withFrameNanos { it }
            val t = (now - start) / 650e6f
            if (t >= 1f) break
            val tick = ((now - start) / 50_000_000L).toInt()
            shown = buildString {
                text.forEachIndexed { i, ch ->
                    val reveal = i.toFloat() / text.length.coerceAtLeast(1) * 0.65f + jitter[i] * 0.35f
                    append(if (ch.isWhitespace() || t >= reveal) ch else GLYPHS[(i * 31 + tick * 7919).mod(GLYPHS.length)])
                }
            }
        }
        shown = text
        scrambling = false
    }
    Text(
        shown,
        modifier = modifier,
        style = style,
        color = if (scrambling) color.copy(alpha = 0.85f) else color,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
    )
}

/**
 * Главная кнопка: на паузе — скруглённый квадрат, играет — «печенье» MD3E, медленно
 * вращается; переход между формами — морфинг. При нажатии чуть сжимается.
 */
@Composable
fun PlayButton(playing: Boolean, size: Dp, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val morph = remember { Morph(MaterialShapes.Square, MaterialShapes.Cookie9Sided) }
    val progress by animateFloatAsState(
        if (playing) 1f else 0f,
        MaterialTheme.motionScheme.defaultSpatialSpec(),
        label = "morph",
    )
    val spin = remember { Animatable(0f) }
    LaunchedEffect(playing) {
        if (playing) {
            while (true) spin.animateTo(spin.value + 360f, tween(14_000, easing = LinearEasing))
        }
    }
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val scale by animateFloatAsState(
        if (pressed) 0.88f else 1f,
        MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "press",
    )
    val color = MaterialTheme.colorScheme.primary
    Box(
        modifier
            .size(size)
            .graphicsLayer { scaleX = scale; scaleY = scale }
            .clickable(interaction, indication = null, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .graphicsLayer { rotationZ = spin.value * progress }
                .drawBehind {
                    val path = morph.toPath(progress)
                    path.transform(Matrix().apply { scale(this@drawBehind.size.width, this@drawBehind.size.height) })
                    drawPath(path, color)
                },
        )
        Icon(
            if (playing) Ic.Pause else Ic.Play,
            contentDescription = if (playing) "Пауза" else "Играть",
            tint = MaterialTheme.colorScheme.onPrimary,
            modifier = Modifier.size(size * 0.42f),
        )
    }
}

/** Круглая кнопка-иконка, при нажатии форма «сжимается» в скруглённый квадрат (MD3E). */
@Composable
fun ShapeIconButton(
    icon: ImageVector,
    description: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 56.dp,
    container: Color = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.7f),
    tint: Color = MaterialTheme.colorScheme.onSurface,
) {
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val corner by animateFloatAsState(
        if (pressed) 30f else 50f,
        MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "corner",
    )
    Box(
        modifier
            .size(size)
            .clip(RoundedCornerShape(corner.toInt()))
            .background(container)
            .clickable(interaction, indication = null, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, description, tint = tint, modifier = Modifier.size(size * 0.45f))
    }
}

/** «Эквалайзер» у играющего трека: три столбика, на паузе замирают. */
@Composable
fun EqBars(playing: Boolean, color: Color, modifier: Modifier = Modifier) {
    val t = rememberInfiniteTransition(label = "eq")
    val bars = listOf(0, 1, 2).map { i ->
        t.animateFloat(
            0.25f,
            1f,
            infiniteRepeatable(tween(420 + i * 130, easing = LinearEasing), RepeatMode.Reverse),
            label = "bar$i",
        )
    }
    Row(modifier.size(18.dp), horizontalArrangement = Arrangement.spacedBy(2.dp), verticalAlignment = Alignment.Bottom) {
        bars.forEach { b ->
            Box(
                Modifier
                    .width(4.dp)
                    .fillMaxHeight(if (playing) b.value else 0.3f)
                    .clip(RoundedCornerShape(2.dp))
                    .background(color),
            )
        }
    }
}

/**
 * Позиция, которая идёт плавно между редкими (4 раза в секунду) обновлениями из ядра.
 */
@Composable
fun smoothPosition(s: PlayerState): Long {
    var base by remember { mutableLongStateOf(s.positionMs.toLong()) }
    var at by remember { mutableLongStateOf(0L) }
    var now by remember { mutableLongStateOf(0L) }
    LaunchedEffect(s.positionMs, s.track?.id) {
        base = s.positionMs.toLong()
        at = withFrameNanos { it }
    }
    val running = !s.paused && !s.loading
    LaunchedEffect(running) {
        while (running) now = withFrameNanos { it }
    }
    return if (running && now > at) base + (now - at) / 1_000_000 else base
}

/** Перемотка MD3E: волнистая полоса (волна — когда играет), тап и перетаскивание. */
@Composable
fun WavySeek(positionMs: Long, durationMs: Long, playing: Boolean, onSeek: (Long) -> Unit, modifier: Modifier = Modifier) {
    val dur = durationMs.coerceAtLeast(1)
    var drag by remember { mutableStateOf<Float?>(null) }
    var width by remember { mutableFloatStateOf(1f) }
    val frac = drag ?: (positionMs.toFloat() / dur).coerceIn(0f, 1f)
    val amp by animateFloatAsState(if (playing && drag == null) 1f else 0f, tween(400), label = "amp")
    Column(modifier) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(36.dp)
                .pointerInput(dur) {
                    width = size.width.toFloat()
                    detectTapGestures { o -> onSeek(((o.x / size.width).coerceIn(0f, 1f) * dur).toLong()) }
                }
                .pointerInput(dur) {
                    detectHorizontalDragGestures(
                        onDragStart = { o -> drag = (o.x / size.width).coerceIn(0f, 1f) },
                        onDragEnd = {
                            drag?.let { onSeek((it * dur).toLong()) }
                            drag = null
                        },
                        onDragCancel = { drag = null },
                    ) { change, _ -> drag = (change.position.x / size.width).coerceIn(0f, 1f) }
                },
            contentAlignment = Alignment.Center,
        ) {
            LinearWavyProgressIndicator(
                progress = { frac },
                modifier = Modifier.fillMaxWidth().height(14.dp),
                color = MaterialTheme.colorScheme.primary,
                trackColor = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.18f),
                amplitude = { amp },
            )
        }
        Row(Modifier.fillMaxWidth()) {
            Text(time((frac * dur).toLong()), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.weight(1f))
            Text(time(durationMs), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** Картинка по https; пока грузится — тонированная подложка. */
@Composable
fun Art(app: YaApp, url: String?, modifier: Modifier, corner: Dp = 12.dp) {
    val bmp by produceState<Bitmap?>(app.cached(url), url) { value = url?.let { app.cover(it) } }
    Box(modifier.clip(RoundedCornerShape(corner)).background(MaterialTheme.colorScheme.surfaceContainerHighest)) {
        bmp?.let {
            Image(it.asImageBitmap(), null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        }
    }
}

fun time(ms: Long): String {
    val s = ms / 1000
    return "%d:%02d".format(s / 60, s % 60)
}
