package xyz.neodustry.yamusicrs

import android.graphics.RuntimeShader
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
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
import kotlin.math.min
import kotlin.math.sin

/**
 * Блоб из TUI (`render/blob.wgsl`): плоские пастельные метаболы, острова цвета с мягкими
 * стыками, тело растворяется к краю. Те же шары и та же формула, только на AGSL.
 */
private const val BLOB_AGSL = """
uniform float2 size;
uniform float t;
uniform float e;
uniform half4 c0;
uniform half4 c1;
uniform half4 c2;
uniform half4 c3;

float3 ball(int k, float t, float e) {
    float fk = float(k);
    if (k < 5) {
        float amp = 0.26 + 0.10 * e;
        float2 c = float2(amp * sin(t * (0.23 + 0.06 * fk) + fk * 2.1),
                          amp * 0.9 * sin(t * (0.17 + 0.05 * fk) + fk * 1.3 + 1.2));
        return float3(c, 0.24 + 0.05 * sin(t * 0.31 + fk * 1.7));
    }
    float amp = 0.55 + 0.08 * sin(t * 0.4 + fk);
    float a = t * (0.45 + 0.1 * fk) + fk * 3.0;
    return float3(amp * cos(a), amp * 0.8 * sin(a * 1.3), 0.09);
}

half3 pick(int k) {
    int m = k - (k / 4) * 4;
    if (m == 0) { return c0.rgb; }
    if (m == 1) { return c1.rgb; }
    if (m == 2) { return c2.rgb; }
    return c3.rgb;
}

half4 main(float2 p) {
    float2 uv = p / size;
    float aspect = size.x / size.y;
    float2 q = uv * 2.0 - 1.0;
    if (aspect > 1.0) { q.x *= aspect; } else { q.y /= aspect; }
    float f = 0.0;
    float wsum = 0.0;
    float3 acc = float3(0.0);
    for (int k = 0; k < 7; k++) {
        float3 b = ball(k, t, e);
        float2 d = q - b.xy;
        float d2 = max(dot(d, d), 1e-4);
        float v = b.z * b.z / d2;
        f += v;
        float w = v * v * v;
        wsum += w;
        acc += w * float3(pick(k));
    }
    float3 col = clamp(acc / max(wsum, 1e-6), 0.0, 1.0);
    float body = smoothstep(0.30, 1.35, f);
    float edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    float a = body * 0.94 * smoothstep(0.0, 0.12, edge);
    return half4(half3(col * a), half(a));
}
"""

/** Живой фон из цветов обложки (`Pal.blob`); `playing` — быстрее и с большим размахом. */
@Composable
fun Blob(colors: List<Color>, playing: Boolean, modifier: Modifier = Modifier) {
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
    if (Build.VERSION.SDK_INT >= 33) {
        ShaderBlob(c, phase, energy, modifier)
    } else {
        GradientBlob(c, phase, energy, modifier)
    }
}

@RequiresApi(33)
@Composable
private fun ShaderBlob(c: List<Color>, phase: Float, energy: Float, modifier: Modifier) {
    val shader = remember { RuntimeShader(BLOB_AGSL) }
    val brush = remember(shader) { ShaderBrush(shader) }
    Canvas(modifier) {
        shader.setFloatUniform("size", size.width, size.height)
        shader.setFloatUniform("t", phase)
        shader.setFloatUniform("e", energy)
        listOf("c0", "c1", "c2", "c3").forEachIndexed { i, n ->
            val col = c[i % c.size]
            shader.setFloatUniform(n, col.red, col.green, col.blue, 1f)
        }
        drawRect(brush)
    }
}

/** Без AGSL (Android < 13): те же основные шары мягкими радиальными пятнами. */
@Composable
private fun GradientBlob(c: List<Color>, phase: Float, energy: Float, modifier: Modifier) {
    Canvas(modifier) {
        val unit = min(size.width, size.height) / 2f
        val center = Offset(size.width / 2f, size.height / 2f)
        for (k in 0 until 5) {
            val amp = 0.26f + 0.10f * energy
            val x = amp * sin(phase * (0.23f + 0.06f * k) + k * 2.1f)
            val y = amp * 0.9f * sin(phase * (0.17f + 0.05f * k) + k * 1.3f + 1.2f)
            val r = (0.24f + 0.05f * sin(phase * 0.31f + k * 1.7f)) * 2.4f
            val o = center + Offset(x * unit, y * unit)
            drawCircle(
                Brush.radialGradient(listOf(c[k % c.size].copy(alpha = 0.85f), Color.Transparent), o, r * unit),
                radius = r * unit,
                center = o,
            )
        }
    }
}
