package xyz.neodustry.yamusicrs

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.mockito.Mockito
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], application = android.app.Application::class, qualifiers = "w411dp-h891dp-xxhdpi")
class ListJumpsShots {
    @get:Rule val rule = createComposeRule()

    @Test
    fun jumps() {
        val app = Mockito.mock(YaApp::class.java)
        rule.mainClock.autoAdvance = false
        rule.setContent {
            MaterialTheme {
                CompositionLocalProvider(LocalNow provides NowMark("x", true)) {
                    Box(Modifier.fillMaxSize()) {
                        val st = rememberLazyListState()
                        LazyColumn(Modifier.fillMaxSize().testTag("list"), state = st) {
                            items(80) { Text("Трек $it", Modifier.size(width = 300.dp, height = 56.dp)) }
                        }
                        ListJumps(app, st, 80, 0, now = 3, nowThumb = null)
                    }
                }
            }
        }
        rule.mainClock.advanceTimeBy(100)
        rule.onRoot().shot("jumps_0_idle")
        rule.onNodeWithTag("list").performTouchInput { swipeUp(durationMillis = 100) }
        for ((i, t) in listOf(60L, 120L, 250L, 400L).withIndex()) {
            rule.mainClock.advanceTimeBy(t)
            rule.onRoot().shot("jumps_${i + 1}_t$t")
        }
    }

    /** Кнопка играющего трека: выезжает, когда трек ушёл с экрана, и уезжает, когда вернулся. */
    @Test
    fun nowButton() {
        val app = Mockito.mock(YaApp::class.java)
        rule.mainClock.autoAdvance = false
        rule.setContent {
            MaterialTheme {
                CompositionLocalProvider(LocalNow provides NowMark("x", true)) {
                    Box(Modifier.fillMaxSize()) {
                        val st = rememberLazyListState()
                        LazyColumn(Modifier.fillMaxSize().testTag("list"), state = st) {
                            items(80) { Text("Трек $it", Modifier.size(width = 300.dp, height = 56.dp)) }
                        }
                        ListJumps(app, st, 80, 0, now = 3, nowThumb = null)
                    }
                }
            }
        }
        rule.mainClock.advanceTimeBy(50)
        // Небольшой свайп: столбик проснулся, трек 3 ещё на экране — средней кнопки нет.
        rule.onNodeWithTag("list").performTouchInput { swipeUp(startY = bottom * 0.5f, endY = bottom * 0.45f, durationMillis = 60) }
        rule.mainClock.advanceTimeBy(500)
        rule.onRoot().shot("now_0_visible")
        // Большой свайп: трек 3 ушёл — кнопка выезжает.
        rule.onNodeWithTag("list").performTouchInput { swipeUp(durationMillis = 100) }
        for ((i, t) in listOf(0L, 60L, 80L, 160L, 400L).withIndex()) {
            rule.mainClock.advanceTimeBy(t)
            rule.onRoot().shot("now_${i + 1}_out_t$t")
        }
    }
}
