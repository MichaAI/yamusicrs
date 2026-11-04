package xyz.neodustry.yamusicrs

import android.graphics.Bitmap
import androidx.compose.ui.graphics.Color
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

/**
 * Палитра под обложку — тот же алгоритм, что в TUI (`ui/theme.rs`): median cut по обложке
 * 64×64, похожие корзины сливаются, акцент — самый «характерный» цвет, для блоба — до
 * четырёх заметно разных.
 */
data class Pal(
    val accent: Color,
    val accent2: Color,
    val surface: Color,
    val blob: List<Color>,
) {
    companion object {
        /** Фирменный жёлтый Музыки + тёплый оранжевый, как в TUI. */
        val Default = Pal(
            accent = Color(0xFFFFCC00),
            accent2 = Color(0xFFFF6B3D),
            surface = Color(0xFF2A2620),
            blob = listOf(Color(0xFFFFCC00), Color(0xFFFF6B3D), Color(0xFFF02A4F), Color(0xFF7A5ADC)),
        )

        fun from(bmp: Bitmap): Pal {
            val small = Bitmap.createScaledBitmap(bmp, 64, 64, true)
            val px = ArrayList<FloatArray>(64 * 64)
            for (y in 0 until 64) for (x in 0 until 64) {
                val c = small.getPixel(x, y)
                if ((c ushr 24) == 0) continue
                px += floatArrayOf(((c shr 16) and 0xff).toFloat(), ((c shr 8) and 0xff).toFloat(), (c and 0xff).toFloat())
            }
            if (px.isEmpty()) return Default
            val buckets = ArrayList<FloatArray>()
            medianCut(px, 0, 4, buckets)
            val clusters = ArrayList<Pair<FloatArray, Float>>()
            for (c in buckets) {
                val i = clusters.indexOfFirst { dist(it.first, c) < 28f * 28f }
                if (i < 0) {
                    clusters += c.copyOf() to 1f
                } else {
                    val (k, n) = clusters[i]
                    for (j in 0..2) k[j] = (k[j] * n + c[j]) / (n + 1f)
                    clusters[i] = k to n + 1f
                }
            }
            val ranked = clusters.map { (c, n) ->
                val hsl = hsl(c)
                val usable = if (hsl[2] < 0.08f || hsl[2] > 0.94f) 0.1f else 1f
                c to sqrt(n) * (0.12f + hsl[1]) * usable
            }.sortedByDescending { it.second }
            val dominant = ranked.firstOrNull()?.first ?: floatArrayOf(255f, 204f, 0f)
            val accent = tuned(dominant, 0.45f, 0.58f, 0.72f)
            val h0 = hsl(dominant)[0]
            val accent2 = ranked.drop(1).map { it.first }.firstOrNull { c ->
                val (h1, s1) = hsl(c).let { it[0] to it[1] }
                val dh = min(abs(h1 - h0), 1f - abs(h1 - h0))
                dh > 0.06f && s1 > 0.25f
            }?.let { tuned(it, 0.4f, 0.5f, 0.68f) } ?: rotate(accent, 0.08f)
            val blob = ArrayList<FloatArray>()
            for ((c, _) in ranked) {
                val t = tuned(c, 0.30f, 0.42f, 0.70f)
                if (blob.all { dist(it, t) > 45f * 45f }) blob += t
                if (blob.size == 4) break
            }
            for (shift in floatArrayOf(0.09f, -0.09f, 0.18f, -0.18f)) if (blob.size < 4) blob += rotate(accent, shift)
            return Pal(
                accent = accent.color(),
                accent2 = accent2.color(),
                surface = tuned(dominant, 0.25f, 0.12f, 0.16f).color(),
                blob = blob.map { it.color() },
            )
        }

        private fun FloatArray.color() = Color(this[0] / 255f, this[1] / 255f, this[2] / 255f)

        private fun dist(a: FloatArray, b: FloatArray) =
            (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]) + (a[2] - b[2]) * (a[2] - b[2])

        private fun medianCut(px: MutableList<FloatArray>, depth: Int, maxDepth: Int, out: MutableList<FloatArray>) {
            if (px.isEmpty()) return
            if (depth >= maxDepth || px.size < 2) {
                val s = FloatArray(3)
                for (p in px) for (j in 0..2) s[j] += p[j]
                out += floatArrayOf(s[0] / px.size, s[1] / px.size, s[2] / px.size)
                return
            }
            val ch = (0..2).maxBy { i -> px.maxOf { it[i] } - px.minOf { it[i] } }
            px.sortBy { it[ch] }
            val mid = px.size / 2
            medianCut(px.subList(0, mid), depth + 1, maxDepth, out)
            medianCut(px.subList(mid, px.size), depth + 1, maxDepth, out)
        }

        private fun hsl(c: FloatArray): FloatArray {
            val r = c[0] / 255f
            val g = c[1] / 255f
            val b = c[2] / 255f
            val mx = max(r, max(g, b))
            val mn = min(r, min(g, b))
            val l = (mx + mn) / 2f
            if (mx - mn < 1e-6f) return floatArrayOf(0f, 0f, l)
            val d = mx - mn
            val s = if (l > 0.5f) d / (2f - mx - mn) else d / (mx + mn)
            val h = when (mx) {
                r -> (g - b) / d + (if (g < b) 6f else 0f)
                g -> (b - r) / d + 2f
                else -> (r - g) / d + 4f
            }
            return floatArrayOf(h / 6f, s, l)
        }

        private fun fromHsl(h: Float, s: Float, l: Float): FloatArray {
            if (s == 0f) return floatArrayOf(l * 255f, l * 255f, l * 255f)
            val q = if (l < 0.5f) l * (1f + s) else l + s - l * s
            val p = 2f * l - q
            fun f(t0: Float): Float {
                val t = ((t0 % 1f) + 1f) % 1f
                return when {
                    t < 1f / 6f -> p + (q - p) * 6f * t
                    t < 0.5f -> q
                    t < 2f / 3f -> p + (q - p) * (2f / 3f - t) * 6f
                    else -> p
                }
            }
            return floatArrayOf(f(h + 1f / 3f) * 255f, f(h) * 255f, f(h - 1f / 3f) * 255f)
        }

        private fun tuned(c: FloatArray, sMin: Float, lMin: Float, lMax: Float): FloatArray {
            val (h, s, l) = hsl(c).let { Triple(it[0], it[1], it[2]) }
            return fromHsl(h, max(s, sMin), l.coerceIn(lMin, lMax))
        }

        private fun rotate(c: FloatArray, by: Float): FloatArray {
            val (h, s, l) = hsl(c).let { Triple(it[0], it[1], it[2]) }
            return fromHsl(h + by, s, l)
        }
    }
}
