@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package xyz.neodustry.yamusicrs

import android.graphics.Bitmap
import android.graphics.RuntimeShader
import android.os.Build
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.platform.LocalDensity
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
fun PlayButton(playing: Boolean, size: Dp, onClick: () -> Unit, modifier: Modifier = Modifier, halo: Boolean = false) {
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
    val ring = MaterialTheme.colorScheme.background
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
                    val w = this@drawBehind.size.width
                    val h = this@drawBehind.size.height
                    // halo — тёмный ореол той же формы: кнопка того же цвета, что блоб под ней,
                    // и без него сливается с пятнами.
                    if (halo) {
                        val k = 1.22f
                        val ringPath = morph.toPath(progress)
                        ringPath.transform(Matrix().apply {
                            translate(w * (1 - k) / 2, h * (1 - k) / 2)
                            scale(w * k, h * k)
                        })
                        drawPath(ringPath, ring.copy(alpha = 0.55f))
                    }
                    val path = morph.toPath(progress)
                    path.transform(Matrix().apply { scale(w, h) })
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
 * Позиция трека между редкими (4 раза в секунду) обновлениями ядра. Читать [pos] только
 * при отрисовке или в `derivedStateOf`: тогда каждый кадр перерисовывается лишь то, что её
 * рисует, без перекомпоновки экрана.
 */
class PlayClock {
    internal val frame = mutableLongStateOf(0L)
    internal var base = 0L
    internal var at = 0L
    internal var running = false

    fun pos(): Long {
        val now = frame.longValue
        return if (running && now > at) base + (now - at) / 1_000_000 else base
    }
}

@Composable
fun rememberPlayClock(s: PlayerState): PlayClock {
    val c = remember { PlayClock() }
    val running = !s.paused && !s.loading && s.track != null
    LaunchedEffect(s.positionMs, s.track?.id, running) {
        c.base = s.positionMs.toLong()
        c.at = System.nanoTime()
        c.running = running
        c.frame.longValue = c.at
    }
    LaunchedEffect(running) {
        while (running) withFrameNanos { c.frame.longValue = it }
    }
    return c
}

/**
 * Волна активной части, как в quickshell (`assets/shaders/wave.frag`) и у MD3E: синусоида
 * толщиной `thick` со скруглёнными концами; цвет — градиент c0 → c1, смешанный в OkLab.
 */
private const val WAVE_AGSL = """
uniform float2 size;
uniform float amp;
uniform float wavelength;
uniform float phase;
uniform float thick;
uniform half4 c0;
uniform half4 c1;

const float TAU = 6.28318530718;

float3 toLinear(float3 c) { return mix(c / 12.92, pow((c + 0.055) / 1.055, float3(2.4)), step(0.04045, c)); }
float3 toSrgb(float3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, float3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
float3 oklab(float3 c) {
    float3 l = toLinear(c);
    float3 lms = pow(max(float3(dot(l, float3(0.4122214708, 0.5363325363, 0.0514459929)),
                                dot(l, float3(0.2119034982, 0.6806995451, 0.1073969566)),
                                dot(l, float3(0.0883024619, 0.2817188376, 0.6299787005))), 0.0), float3(1.0 / 3.0));
    return float3(dot(lms, float3(0.2104542553, 0.7936177850, -0.0040720468)),
                  dot(lms, float3(1.9779984951, -2.4285922050, 0.4505937099)),
                  dot(lms, float3(0.0259040371, 0.7827717662, -0.8086757660)));
}
float3 fromOklab(float3 c) {
    float3 lms = float3(dot(c, float3(1.0, 0.3963377774, 0.2158037573)),
                        dot(c, float3(1.0, -0.1055613458, -0.0638541728)),
                        dot(c, float3(1.0, -0.0894841775, -1.2914855480)));
    lms = lms * lms * lms;
    return toSrgb(float3(dot(lms, float3(4.0767416621, -3.3077115913, 0.2309699292)),
                         dot(lms, float3(-1.2684380046, 2.6097574011, -0.3413193965)),
                         dot(lms, float3(-0.0041960863, -0.7034186147, 1.7076147010))));
}
float wave(float x) { return size.y * 0.5 + amp * sin(TAU * (x / wavelength - phase)); }
float slopeAt(float x) { return amp * TAU / wavelength * cos(TAU * (x / wavelength - phase)); }

half4 main(float2 p) {
    float r = thick * 0.5;
    float x0 = r + 0.5;
    float x1 = size.x - r - 0.5;
    float2 e0 = float2(x0, wave(x0));
    float2 e1 = float2(x1, wave(x1));
    float2 t0 = normalize(float2(1.0, slopeAt(x0)));
    float2 t1 = normalize(float2(1.0, slopeAt(x1)));
    float d;
    if (dot(p - e0, t0) < 0.0) {
        d = distance(p, e0);
    } else if (dot(p - e1, t1) > 0.0) {
        d = distance(p, e1);
    } else {
        float x = clamp(p.x, x0, x1);
        float sl = slopeAt(x);
        d = abs(p.y - wave(x)) / sqrt(1.0 + sl * sl);
    }
    float a = clamp(r - d + 0.5, 0.0, 1.0);
    float t = clamp((p.x - x0) / max(x1 - x0, 1.0), 0.0, 1.0);
    float3 c = fromOklab(mix(oklab(float3(c0.rgb)), oklab(float3(c1.rgb)), t));
    return half4(half3(c * a), half(a));
}
"""

/**
 * Перемотка как в quickshell: активная часть — волна (шейдер, на Android < 13 — плоская),
 * дальше плоская дорожка, стоп в конце, ручка худеет при нажатии. Волна бежит на длину
 * в секунду, амплитуда плавно появляется и гаснет; всё это — только перерисовка.
 */
@Composable
fun WavySeek(clock: PlayClock, durationMs: Long, playing: Boolean, onSeek: (Long) -> Unit, modifier: Modifier = Modifier) {
    val dur = durationMs.coerceAtLeast(1)
    var drag by remember { mutableStateOf<Float?>(null) }
    val amp = remember { Animatable(0f) }
    val moving = playing && drag == null
    LaunchedEffect(moving) { amp.animateTo(if (moving) 1f else 0f, tween(500)) }
    // Фаза волны — свой кадровый счётчик: бежит, пока волна видна.
    val phase = remember { mutableFloatStateOf(0f) }
    LaunchedEffect(moving) {
        var last = 0L
        while (moving || amp.value > 0f) {
            withFrameNanos { now ->
                if (last != 0L) phase.floatValue = (phase.floatValue + (now - last) / 1e9f) % 1f
                last = now
            }
        }
    }
    val shader = remember { if (Build.VERSION.SDK_INT >= 33) RuntimeShader(WAVE_AGSL) else null }
    val c0 = MaterialTheme.colorScheme.primary
    val c1 = MaterialTheme.colorScheme.secondary
    val track = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.16f)
    val density = LocalDensity.current
    val (trackH, gap, waveAmp, wavelength) = with(density) { listOf(5.dp.toPx(), 6.dp.toPx(), 3.dp.toPx(), 36.dp.toPx()) }
    Column(modifier) {
        Box(
            Modifier
                .fillMaxWidth()
                .height(32.dp)
                .pointerInput(dur) {
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
                    ) { change, _ ->
                        change.consume()
                        drag = (change.position.x / size.width).coerceIn(0f, 1f)
                    }
                }
                .drawBehind {
                    val frac = drag ?: (clock.pos().toFloat() / dur).coerceIn(0f, 1f)
                    val cy = size.height / 2f
                    val cx = frac * size.width
                    val handleW = if (drag != null) 2.dp.toPx() else 4.dp.toPx()
                    // Неактивная дорожка и стоп в конце
                    val inactiveStart = (cx + gap).coerceAtMost(size.width)
                    if (inactiveStart < size.width) {
                        drawRoundRect(
                            track,
                            topLeft = Offset(inactiveStart, cy - trackH / 2),
                            size = Size(size.width - inactiveStart, trackH),
                            cornerRadius = CornerRadius(trackH / 2),
                        )
                    }
                    drawCircle(c1, radius = 2.dp.toPx(), center = Offset(size.width - 2.dp.toPx(), cy))
                    // Активная часть: волна до ручки
                    val activeEnd = (cx - gap).coerceAtLeast(0f)
                    if (activeEnd > trackH) {
                        val a = waveAmp * amp.value
                        if (shader != null && Build.VERSION.SDK_INT >= 33) {
                            val h = 2 * waveAmp + trackH + 2
                            shader.setFloatUniform("size", activeEnd, h)
                            shader.setFloatUniform("amp", a)
                            shader.setFloatUniform("wavelength", wavelength)
                            shader.setFloatUniform("phase", phase.floatValue)
                            shader.setFloatUniform("thick", trackH)
                            shader.setFloatUniform("c0", c0.red, c0.green, c0.blue, 1f)
                            shader.setFloatUniform("c1", c1.red, c1.green, c1.blue, 1f)
                            translate(0f, cy - h / 2) {
                                drawRect(ShaderBrush(shader), size = Size(activeEnd, h))
                            }
                        } else {
                            drawRoundRect(
                                Brush.horizontalGradient(listOf(c0, c1), endX = activeEnd),
                                topLeft = Offset(0f, cy - trackH / 2),
                                size = Size(activeEnd, trackH),
                                cornerRadius = CornerRadius(trackH / 2),
                            )
                        }
                    }
                    // Ручка
                    drawRoundRect(
                        c1,
                        topLeft = Offset(cx - handleW / 2, cy - 11.dp.toPx()),
                        size = Size(handleW, 22.dp.toPx()),
                        cornerRadius = CornerRadius(handleW / 2),
                    )
                },
        )
        Row(Modifier.fillMaxWidth()) {
            val shown by remember(dur) { derivedStateOf { (drag?.let { (it * dur).toLong() } ?: clock.pos()) / 1000 } }
            Text(time(shown * 1000), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.weight(1f))
            Text(time(durationMs), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** Картинка по https; пока грузится — тонированная подложка. */
@Composable
fun Art(app: YaApp, url: String?, modifier: Modifier, corner: Dp = 12.dp, plain: Boolean = false) {
    val bmp by produceState<Bitmap?>(app.cached(url), url) { value = url?.let { app.cover(it) } }
    val img = remember(bmp) { bmp?.asImageBitmap() }
    // plain — картинка как есть (иконки агентов волны с прозрачностью): без подложки и скругления.
    // Фото (непрозрачный угол) скругляем и в plain-режиме; иконки с прозрачностью — как есть.
    val photo = remember(bmp) { bmp?.let { (it.getPixel(0, 0) ushr 24) == 0xFF } ?: false }
    val box = when {
        !plain -> modifier.clip(RoundedCornerShape(corner)).background(MaterialTheme.colorScheme.surfaceContainerHighest)
        photo -> modifier.clip(RoundedCornerShape(22.dp))
        else -> modifier
    }
    Box(box) {
        img?.let {
            Image(it, null, contentScale = if (plain) ContentScale.Fit else ContentScale.Crop, modifier = Modifier.fillMaxSize())
        }
    }
}

fun time(ms: Long): String {
    val s = ms / 1000
    return "%d:%02d".format(s / 60, s % 60)
}
