package xyz.neodustry.yamusicrs

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Геометрия кругового перехода обложек: обычное фото → обычное и обычное → вырезка во всю ширину. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], application = android.app.Application::class, qualifiers = "w411dp-h700dp-xxhdpi")
class RevealShots {
    @get:Rule val rule = createComposeRule()

    private fun run(tag: String, wide: Boolean) {
        var f by mutableFloatStateOf(0f)
        val side = 300.dp
        val small = 100.dp
        val full = 411.dp
        rule.setContent {
            val smallPx = with(androidx.compose.ui.platform.LocalDensity.current) { small.toPx() }
            Box(Modifier.fillMaxSize().background(Color.White)) {
                // Рамка шириной с экран — видно, дошёл ли круг до края.
                Box(Modifier.fillMaxWidth().height(side + small / 2).border(1.dp, Color.LightGray))
                Box(Modifier.size(side, side + small / 2).align(Alignment.TopCenter)) {
                    // Старая: фото в квадрате со скруглением (снаружи круга).
                    Box(
                        Modifier.matchParentSize().circleReveal({ f }, { true }, smallPx, if (wide) full else null, inverse = true),
                        contentAlignment = Alignment.TopCenter,
                    ) { Box(Modifier.size(side).clip(RoundedCornerShape(48.dp)).background(Color(0xFF9FA8DA))) }
                    Box(
                        Modifier.matchParentSize().circleReveal({ f }, { true }, smallPx, if (wide) full else null),
                        contentAlignment = Alignment.TopCenter,
                    ) {
                        if (wide) {
                            // Новая: вырезка на всю ширину экрана.
                            // Полупрозрачная, как вырезка с прозрачным фоном: старое сквозь неё видно быть не должно.
                            Box(Modifier.requiredSize(full, side).background(Color(0x66EF5350)))
                        } else {
                            Box(Modifier.size(side).clip(RoundedCornerShape(48.dp)).background(Color(0xFFEF5350)))
                        }
                    }
                }
            }
        }
        for (p in listOf(0.2f, 0.4f, 0.6f, 0.8f)) {
            rule.runOnUiThread { f = p }
            rule.waitForIdle()
            rule.onRoot().shot("reveal_${tag}_${(p * 100).toInt()}")
        }
    }

    @Test fun normalToNormal() = run("plain", wide = false)
    @Test fun normalToCutout() = run("cutout", wide = true)
}
