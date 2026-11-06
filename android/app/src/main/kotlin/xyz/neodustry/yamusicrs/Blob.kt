package xyz.neodustry.yamusicrs

import android.graphics.RuntimeShader
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ShaderBrush
import androidx.compose.ui.graphics.drawscope.clipPath
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.findRootCoordinates
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.composed
import kotlin.math.min
import kotlin.math.sin

/**
 * Блоб из TUI (`render/blob.wgsl`): плоские пастельные метаболы, острова цвета с мягкими
 * стыками, тело растворяется к краю. Те же шары и та же формула, только на AGSL.
 */
private const val BLOB_AGSL = """
// Всё, что одинаково для всех пикселей, считается на CPU раз за кадр:
// q = p·qs + qo (пропорции экрана и zoom), uv = p·inv, шары (x, y, r²) и их цвета.
uniform float2 qs;
uniform float2 qo;
uniform float2 inv;
uniform float3 b0;
uniform float3 b1;
uniform float3 b2;
uniform float3 b3;
uniform float3 b4;
uniform float3 b5;
uniform float3 b6;
uniform half3 k0;
uniform half3 k1;
uniform half3 k2;
uniform half3 k3;

// Вклад шара r²/d². Координаты — float: в half соседние пиксели слиплись бы. Сам вклад
// не больше 0,0841/1e-4 ≈ 841 — в half влезает.
half field(float2 q, float3 b) {
    float2 d = q - b.xy;
    return half(b.z / max(dot(d, d), 1e-4));
}

half4 main(float2 p) {
    float2 q = p * qs + qo;
    half v0 = field(q, b0); half v1 = field(q, b1); half v2 = field(q, b2); half v3 = field(q, b3);
    half v4 = field(q, b4); half v5 = field(q, b5); half v6 = field(q, b6);
    half f = v0 + v1 + v2 + v3 + v4 + v5 + v6;
    // Веса цвета v³, нормированные на f³ (отношение то же): u = v/f ∈ [0, 1], Σu³ ≥ 1/49 —
    // в half ни переполнения у центров, ни провала в ноль на краю.
    half n = 1.0 / max(f, 1e-3);
    half u0 = v0 * n; half u1 = v1 * n; half u2 = v2 * n; half u3 = v3 * n;
    half u4 = v4 * n; half u5 = v5 * n; half u6 = v6 * n;
    half w0 = u0 * u0 * u0; half w1 = u1 * u1 * u1; half w2 = u2 * u2 * u2; half w3 = u3 * u3 * u3;
    half w4 = u4 * u4 * u4; half w5 = u5 * u5 * u5; half w6 = u6 * u6 * u6;
    half wsum = w0 + w1 + w2 + w3 + w4 + w5 + w6;
    // Цвет шара k — k % 4 палитры (как в TUI).
    half3 acc = (w0 + w4) * k0 + (w1 + w5) * k1 + (w2 + w6) * k2 + w3 * k3;
    half3 col = clamp(acc / wsum, 0.0, 1.0);
    half body = smoothstep(0.30, 1.35, f);
    float2 uv = p * inv;
    half edge = half(min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)));
    half a = body * 0.94 * smoothstep(0.0, 0.12, edge);
    return half4(col * a, a);
}
"""

/**
 * Шары блоба на фазе `t` при размахе `e`: 7 × (x, y, r). Основные пять мягко расталкиваются,
 * чтобы не сходились почти вплотную. Считается раз за кадр — не в шейдере на каждый пиксель.
 */
private fun blobBalls(t: Float, e: Float): FloatArray {
    val b = FloatArray(21)
    for (k in 0 until 7) {
        val fk = k.toFloat()
        if (k < 5) {
            val amp = 0.26f + 0.10f * e
            b[k * 3] = amp * sin(t * (0.23f + 0.06f * fk) + fk * 2.1f)
            b[k * 3 + 1] = amp * 0.9f * sin(t * (0.17f + 0.05f * fk) + fk * 1.3f + 1.2f)
            b[k * 3 + 2] = 0.24f + 0.05f * sin(t * 0.31f + fk * 1.7f)
        } else {
            val amp = 0.55f + 0.08f * sin(t * 0.4f + fk)
            val a = t * (0.45f + 0.1f * fk) + fk * 3f
            b[k * 3] = amp * kotlin.math.cos(a)
            b[k * 3 + 1] = amp * 0.8f * sin(a * 1.3f)
            b[k * 3 + 2] = 0.09f
        }
    }
    val x0 = FloatArray(5) { b[it * 3] }
    val y0 = FloatArray(5) { b[it * 3 + 1] }
    repeat(2) {
        for (i in 0 until 5) {
            var px = 0f
            var py = 0f
            for (j in 0 until 5) {
                if (i == j) continue
                val dx = b[i * 3] - b[j * 3]
                val dy = b[i * 3 + 1] - b[j * 3 + 1]
                val l = kotlin.math.sqrt(dx * dx + dy * dy)
                val m = 0.75f * (b[i * 3 + 2] + b[j * 3 + 2])
                if (l < m) {
                    var ux: Float
                    var uy: Float
                    if (l > 1e-3f) {
                        ux = dx / l; uy = dy / l
                    } else {
                        val n = kotlin.math.sqrt((i - j) * (i - j) + 0.25f)
                        ux = (i - j) / n; uy = 0.5f / n
                    }
                    val x = (m - l) / m
                    val p = (m - l) * 0.5f * x * (2f - x)
                    px += ux * p
                    py += uy * p
                }
            }
            b[i * 3] += px
            b[i * 3 + 1] += py
        }
    }
    // Расталкивание сдвигает шар не дальше PUSH_MAX — на этом держится граница BLOB_REACH.
    for (i in 0 until 5) {
        val dx = b[i * 3] - x0[i]
        val dy = b[i * 3 + 1] - y0[i]
        val l = kotlin.math.sqrt(dx * dx + dy * dy)
        if (l > PUSH_MAX) {
            b[i * 3] = x0[i] + dx / l * PUSH_MAX
            b[i * 3 + 1] = y0[i] + dy / l * PUSH_MAX
        }
    }
    return b
}

private const val PUSH_MAX = 0.1f

/**
 * Докуда блоб может дотянуться (в координатах шейдера `q`, до деления на zoom), по x и по y.
 * Пиксель прозрачен ровно (smoothstep → 0), где поле f = Σ r²/d² ≤ 0,30. За пределами центров
 * d ≥ |Y| − |yₖ|, значит f ≤ Σ rₖ²/(|Y| − Yₖ)², где Yₖ, rₖ — наибольшие по формулам шаров:
 * основные |x| ≤ 0,36 (+сдвиг), |y| ≤ 0,324 (+сдвиг), r ≤ 0,29; спутники |x| ≤ 0,63, |y| ≤ 0,504,
 * r = 0,09. Правая часть убывает — граница там, где она падает до 0,30.
 */
private val BLOB_REACH: Pair<Float, Float> by lazy {
    fun reach(main: Float, sat: Float): Float {
        val g = { y: Float -> 5 * 0.29f * 0.29f / (y - main) / (y - main) + 2 * 0.09f * 0.09f / (y - sat) / (y - sat) }
        var lo = maxOf(main, sat) + 1e-4f
        var hi = 10f
        repeat(60) {
            val mid = (lo + hi) / 2
            if (g(mid) > 0.30f) lo = mid else hi = mid
        }
        return hi
    }
    reach(0.36f + PUSH_MAX, 0.63f) to reach(0.324f + PUSH_MAX, 0.504f)
}

/** Прямоугольник холста, вне которого блоб гарантированно прозрачен. */
private fun blobBounds(w: Float, h: Float, zoom: Float): androidx.compose.ui.geometry.Rect {
    val (rx, ry) = BLOB_REACH
    val aspect = w / h
    // q = (2·uv − 1), по длинной стороне растянут на aspect, затем / zoom.
    val hu = if (aspect > 1f) rx * zoom / aspect else rx * zoom
    val hv = if (aspect > 1f) ry * zoom else ry * zoom * aspect
    val l = ((1f - hu) / 2f * w).coerceAtLeast(0f)
    val t = ((1f - hv) / 2f * h).coerceAtLeast(0f)
    // С запасом в пиксель на округление.
    return androidx.compose.ui.geometry.Rect(
        (l - 1f).coerceAtLeast(0f), (t - 1f).coerceAtLeast(0f),
        (w - l + 1f).coerceAtMost(w), (h - t + 1f).coerceAtMost(h),
    )
}

/**
 * Непрозрачные элементы над блобом: под ними шейдер не считается. Своё на каждый блоб —
 * иначе элемент с одного экрана продырявил бы блоб другого, лежащего над ним.
 */
class BlobHoles {
    internal val map = androidx.compose.runtime.mutableStateMapOf<Any, androidx.compose.ui.geometry.RoundRect>()
}

/**
 * Элемент непрозрачен (сплошной фон, `corner` — его скругление) — пока `opaque`, блоб под ним
 * не рисуется. `opaque` = false, как только у него альфа < 1 (появление, свайп, переход).
 * Вырез на 1,5 px меньше: сглаженный край полупрозрачен, под ним блоб нужен.
 */
fun Modifier.blobHole(holes: BlobHoles?, corner: androidx.compose.ui.unit.Dp, opaque: Boolean): Modifier =
    if (holes == null) this else this.then(
        androidx.compose.ui.Modifier.composed {
            val key = remember { Any() }
            val density = androidx.compose.ui.platform.LocalDensity.current
            var coords by remember { androidx.compose.runtime.mutableStateOf<androidx.compose.ui.layout.LayoutCoordinates?>(null) }
            DisposableEffect(key) { onDispose { holes.map.remove(key) } }
            fun put(c: androidx.compose.ui.layout.LayoutCoordinates) {
                if (!opaque || !c.isAttached) {
                    holes.map.remove(key)
                    return
                }
                val inset = 1.5f
                val r = c.boundsInRoot()
                if (r.width <= 2 * inset || r.height <= 2 * inset) return holes.map.remove(key).let {}
                val rad = with(density) { corner.toPx() } + inset
                holes.map[key] = androidx.compose.ui.geometry.RoundRect(
                    r.deflate(inset), androidx.compose.ui.geometry.CornerRadius(rad, rad),
                )
            }
            androidx.compose.runtime.SideEffect { coords?.let(::put) ?: run { if (!opaque) holes.map.remove(key) } }
            androidx.compose.ui.Modifier.onGloballyPositioned { coords = it; put(it) }
        },
    )

/**
 * Живой фон из цветов обложки (`Pal.blob`); `playing` — быстрее и с большим размахом.
 * `zoom` растягивает сами шары, а не картинку: шейдер считает каждый пиксель экрана, так что
 * крупнее — без потери чёткости.
 */
@Composable
fun Blob(colors: List<Color>, playing: Boolean, modifier: Modifier = Modifier, zoom: Float = 1.25f, holes: BlobHoles? = null) {
    when (Viz.kind) {
        "spectrum" -> SpectrumBackdrop(colors, playing, modifier)
        // Оба сразу: блоб — фон, столбики поверх без собственной подложки.
        "both" -> androidx.compose.foundation.layout.Box(modifier) {
            BlobOnly(colors, playing, Modifier.fillMaxSize(), zoom, holes)
            SpectrumBackdrop(colors, playing, Modifier.fillMaxSize(), backdrop = false)
        }
        else -> BlobOnly(colors, playing, modifier, zoom, holes)
    }
}

@Composable
private fun BlobOnly(colors: List<Color>, playing: Boolean, modifier: Modifier, zoom: Float, holes: BlobHoles?) {
    val c = colors.take(4).map { animateColorAsState(it, tween(900), label = "blob").value }
    var phase by remember { mutableFloatStateOf(0f) }
    var speed by remember { mutableFloatStateOf(0.3f) }
    var energy by remember { mutableFloatStateOf(0f) }
    val play by rememberUpdatedState(playing)
    // Как в TUI: «дышит» быстрее, когда играет; скорость и размах догоняют цель плавно.
    LaunchedEffect(Unit) {
        var last = 0L
        while (true) {
            withFrameNanos { now ->
                val dt = if (last == 0L) 0f else ((now - last) / 1e9f).coerceAtMost(0.1f)
                last = now
                val k = min(dt * 2f, 1f)
                speed += ((if (play) 1f else 0.3f) - speed) * k
                energy += ((if (play) 1f else 0f) - energy) * k
                phase += dt * speed
            }
        }
    }
    // Фаза и размах читаются только при отрисовке: блоб не перекомпонуется каждый кадр.
    if (Build.VERSION.SDK_INT >= 33) {
        ShaderBlob(c, { phase }, { energy }, zoom, modifier, holes)
    } else {
        GradientBlob(c, { phase }, { energy }, zoom, modifier)
    }
}

@RequiresApi(33)
private object BlobShaders {
    private val free = ArrayDeque<RuntimeShader>()

    fun take(): RuntimeShader = free.removeFirstOrNull() ?: RuntimeShader(BLOB_AGSL)

    fun give(s: RuntimeShader) {
        free.addLast(s)
    }
}

@RequiresApi(33)
@Composable
private fun ShaderBlob(c: List<Color>, phase: () -> Float, energy: () -> Float, zoom: Float, modifier: Modifier, holes: BlobHoles?) {
    // Компиляция шейдера заметна по времени — берём готовый из пула (их нужно два:
    // вкладка «Волна» и сцена видны одновременно во время перехода).
    val shader = remember { BlobShaders.take() }
    DisposableEffect(shader) { onDispose { BlobShaders.give(shader) } }
    val brush = remember(shader) { ShaderBrush(shader) }
    var me by remember { androidx.compose.runtime.mutableStateOf<androidx.compose.ui.layout.LayoutCoordinates?>(null) }
    val path = remember { androidx.compose.ui.graphics.Path() }
    Canvas(modifier.onGloballyPositioned { me = it }) {
        // q = (2·p/size − 1), по длинной стороне × пропорции, затем / zoom — одно p·qs + qo.
        val w = size.width
        val h = size.height
        val aspect = w / h
        val sx = if (aspect > 1f) aspect else 1f
        val sy = if (aspect > 1f) 1f else 1f / aspect
        shader.setFloatUniform("qs", 2f / w * sx / zoom, 2f / h * sy / zoom)
        shader.setFloatUniform("qo", -sx / zoom, -sy / zoom)
        shader.setFloatUniform("inv", 1f / w, 1f / h)
        val b = blobBalls(phase(), energy())
        for (k in 0 until 7) shader.setFloatUniform("b$k", b[k * 3], b[k * 3 + 1], b[k * 3 + 2] * b[k * 3 + 2])
        for (i in 0 until 4) {
            val col = c[i % c.size]
            shader.setFloatUniform("k$i", col.red, col.green, col.blue)
        }
        // Шейдер — только там, где блоб может быть виден: в пределах его досягаемости
        // и не под непрозрачными элементами.
        val area = blobBounds(size.width, size.height, zoom)
        path.reset()
        val m = me
        val hs = holes?.map?.values
        if (m != null && m.isAttached && !hs.isNullOrEmpty()) {
            val root = m.findRootCoordinates()
            for (h in hs) {
                // Из координат корня — в свои (с учётом масштаба/сдвига слоёв над блобом).
                val tl = m.localPositionOf(root, Offset(h.left, h.top))
                val br = m.localPositionOf(root, Offset(h.right, h.bottom))
                val k = (br.x - tl.x) / h.width
                path.addRoundRect(
                    androidx.compose.ui.geometry.RoundRect(
                        tl.x, tl.y, br.x, br.y,
                        androidx.compose.ui.geometry.CornerRadius(h.topLeftCornerRadius.x * k, h.topLeftCornerRadius.y * k),
                    ),
                )
            }
        }
        if (path.isEmpty) {
            drawRect(brush, area.topLeft, area.size)
        } else {
            clipPath(path, androidx.compose.ui.graphics.ClipOp.Difference) { drawRect(brush, area.topLeft, area.size) }
        }
    }
}

/** Без AGSL (Android < 13): те же основные шары мягкими радиальными пятнами. */
@Composable
private fun GradientBlob(c: List<Color>, phase: () -> Float, energy: () -> Float, zoom: Float, modifier: Modifier) {
    Canvas(modifier) {
        val unit = min(size.width, size.height) / 2f * zoom
        val center = Offset(size.width / 2f, size.height / 2f)
        val b = blobBalls(phase(), energy())
        for (k in 0 until 5) {
            val x = b[k * 3]
            val y = b[k * 3 + 1]
            val r = b[k * 3 + 2] * 2.4f
            val o = center + Offset(x * unit, y * unit)
            drawCircle(
                Brush.radialGradient(listOf(c[k % c.size].copy(alpha = 0.85f), Color.Transparent), o, r * unit),
                radius = r * unit,
                center = o,
            )
        }
    }
}
